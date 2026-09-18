use app_lib::repo::FileKind;
use app_lib::store::{hash_text, StickyStore};
use std::path::PathBuf;

fn date(s: &str) -> chrono::NaiveDate { chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap() }
fn setup(name: &str) -> (PathBuf, StickyStore) {
    let root = std::env::temp_dir().join(format!("sticky-notes-{name}-{}", std::process::id()));
    std::fs::create_dir_all(root.join(".git")).unwrap();
    let store = StickyStore::new(&root);
    (root, store)
}

#[test]
fn missing_day_returns_empty_content_and_zero_version() {
    let (root, store) = setup("missing");
    let view = store.get_notes("default", FileKind::Day(date("2025-12-29"))).unwrap();
    assert_eq!(view.kind, "day");
    assert_eq!(view.date, "2025-12-29");
    assert_eq!(view.days.len(), 1);
    assert_eq!(view.days[0].weekday, "周一");
    assert_eq!(view.days[0].content, "");
    assert_eq!(view.days[0].base_version, 0);
    assert!(!root.join("notes").exists());
}

#[test]
fn multiline_roundtrip_and_update() {
    let (root, store) = setup("roundtrip");
    let day = date("2025-12-29");
    let saved = store.save_note("default", day, "第一行\r\n\t第二行\n", 0).unwrap();
    let view = store.get_notes("default", FileKind::Day(day)).unwrap();
    assert_eq!(view.days[0].content, "第一行\n\t第二行\n");
    assert_eq!(view.days[0].base_version, saved.base_version);
    assert_eq!(saved.base_version, hash_text(&view.days[0].content));
    let json = serde_json::to_value(&view).unwrap();
    assert_eq!(json["days"][0]["base_version"], saved.base_version.to_string());
    let updated = store.save_note("default", day, "更新\n", saved.base_version).unwrap();
    assert_eq!(updated.base_version, hash_text("更新\n"));
    assert_eq!(std::fs::read_to_string(root.join("notes/2025-12-29.md")).unwrap(), "更新\n");
    assert_eq!(store.take_pending_edits(), 2);
}

#[test]
fn stale_writes_and_deletions_are_rejected() {
    let (_, store) = setup("stale");
    let day = date("2025-12-29");
    assert_eq!(store.save_note("default", day, "new", 1).unwrap_err().code, "stale");
    let saved = store.save_note("default", day, "original", 0).unwrap();
    let wrong = saved.base_version.wrapping_add(1);
    for content in ["replacement", " \n"] {
        assert_eq!(store.save_note("default", day, content, wrong).unwrap_err().code, "stale");
    }
    assert_eq!(store.get_notes("default", FileKind::Day(day)).unwrap().days[0].content, "original");
    assert_eq!(store.take_pending_edits(), 1);
}

#[test]
fn whitespace_deletes_file_and_resets_version() {
    let (root, store) = setup("delete");
    let day = date("2025-12-29");
    let saved = store.save_note("default", day, "text", 0).unwrap();
    let deleted = store.save_note("default", day, " \r\n\t", saved.base_version).unwrap();
    assert_eq!(deleted.base_version, 0);
    assert!(!root.join("notes/2025-12-29.md").exists());
    assert_eq!(store.get_notes("default", FileKind::Day(day)).unwrap().days[0].base_version, 0);
    assert_eq!(store.save_note("default", day, "", 0).unwrap().base_version, 0);
    assert_eq!(store.take_pending_edits(), 3);
}

#[test]
fn week_merges_only_nonempty_days_in_monday_order() {
    let (root, store) = setup("week");
    store.save_note("default", date("2025-12-31"), "Wednesday", 0).unwrap();
    store.save_note("default", date("2025-12-29"), "Monday", 0).unwrap();
    std::fs::write(root.join("notes/2025-12-30.md"), " \n\t").unwrap();
    let view = store.get_notes("default", FileKind::Week(date("2026-01-01"))).unwrap();
    assert_eq!(view.kind, "week");
    assert_eq!(view.date, "2026-01-01");
    assert_eq!(view.days.iter().map(|d| d.date.as_str()).collect::<Vec<_>>(), vec!["2025-12-29", "2025-12-31"]);
    assert_eq!(view.days[0].content, "Monday");
    assert_eq!(view.days[1].content, "Wednesday");
    assert_eq!(view.days[1].weekday, "周三");
    assert_eq!(view.days[1].base_version, hash_text("Wednesday"));
}

#[test]
fn control_characters_are_bad_requests() {
    let (root, store) = setup("control");
    assert_eq!(store.save_note("default", date("2025-12-29"), "a\0b", 0).unwrap_err().code, "bad_request");
    assert!(!root.join("notes").exists());
    assert_eq!(store.take_pending_edits(), 0);
}

#[test]
fn oversized_content_is_a_bad_request() {
    let (root, store) = setup("oversized");
    assert_eq!(store.save_note("default", date("2025-12-29"), &"a".repeat(64001), 0).unwrap_err().code, "bad_request");
    assert!(!root.join("notes").exists());
    assert_eq!(store.take_pending_edits(), 0);
}

#[test]
fn notebooks_are_isolated_and_notes_are_watched() {
    let (root, store) = setup("notebooks");
    let book = store.save_notebook(None, "速记本", 0).unwrap();
    let day = date("2025-12-29");
    let before = store.watched_fingerprints(day);
    store.save_note("default", day, "default note", 0).unwrap();
    let after_default = store.watched_fingerprints(day);
    assert_ne!(before, after_default);
    store.save_note(&book.id, day, "other note", 0).unwrap();
    assert_ne!(after_default, store.watched_fingerprints(day));
    assert_eq!(store.get_notes("default", FileKind::Day(day)).unwrap().days[0].content, "default note");
    assert_eq!(store.get_notes(&book.id, FileKind::Day(day)).unwrap().days[0].content, "other note");
    assert!(root.join(format!("notebooks/{}/notes/2025-12-29.md", book.id)).is_file());
}
