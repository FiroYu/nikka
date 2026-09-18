//! 随日速记的路径、文本校验与周日期筛选；不依赖文件系统。
use chrono::{Duration, NaiveDate, Weekday};

pub fn rel_path(date: NaiveDate) -> String {
    format!("notes/{}.md", crate::repo::fmt_date(date))
}

pub fn validate_and_normalize(text: &str) -> Result<String, String> {
    let text = text.replace("\r\n", "\n");
    if text.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
        return Err("速记不能包含换行和制表符之外的控制字符".into());
    }
    if text.len() > 64000 {
        return Err("速记不能超过 64000 字节".into());
    }
    Ok(text)
}

pub fn weekday_full(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "周一",
        Weekday::Tue => "周二",
        Weekday::Wed => "周三",
        Weekday::Thu => "周四",
        Weekday::Fri => "周五",
        Weekday::Sat => "周六",
        Weekday::Sun => "周日",
    }
}

/// 仅生成候选日；调用方检查是否完整七天，以拒绝日期越界。
pub fn week_days(monday: NaiveDate, _today: NaiveDate) -> Vec<NaiveDate> {
    (0..7).filter_map(|offset| monday.checked_add_signed(Duration::days(offset))).collect()
}

/// existing 应含该周全部候选日，缺文件以空串表示，避免收录周外的今天。
pub fn days_to_include(existing: &[(NaiveDate, String)], today: NaiveDate) -> Vec<NaiveDate> {
    existing.iter().filter(|(date, text)| *date == today || !text.trim().is_empty())
        .map(|(date, _)| *date).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(day: u32) -> NaiveDate { NaiveDate::from_ymd_opt(2026, 1, day).unwrap() }
    fn empty_week() -> Vec<(NaiveDate, String)> {
        week_days(date(5), date(1)).into_iter().map(|d| (d, String::new())).collect()
    }

    #[test]
    fn path_and_weekdays() {
        assert_eq!(rel_path(date(1)), "notes/2026-01-01.md");
        let days = [Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri, Weekday::Sat, Weekday::Sun];
        assert_eq!(days.map(weekday_full), ["周一", "周二", "周三", "周四", "周五", "周六", "周日"]);
    }

    #[test]
    fn normalizes_crlf_and_preserves_multiline_text() {
        assert_eq!(validate_and_normalize("a\r\n\t中\n").unwrap(), "a\n\t中\n");
    }

    #[test]
    fn rejects_control_characters() {
        for c in ['\0', '\r', '\u{7f}', '\u{85}'] {
            assert!(validate_and_normalize(&format!("a{c}b")).is_err());
        }
    }

    #[test]
    fn byte_limit_applies_after_normalization() {
        assert!(validate_and_normalize(&"a".repeat(64000)).is_ok());
        assert!(validate_and_normalize(&"a".repeat(64001)).is_err());
        assert!(validate_and_normalize(&"中".repeat(21334)).is_err());
        assert_eq!(validate_and_normalize(&"\r\n".repeat(64000)).unwrap().len(), 64000);
    }

    #[test]
    fn empty_week_includes_nothing() {
        assert!(days_to_include(&empty_week(), date(1)).is_empty());
    }

    #[test]
    fn includes_two_nonempty_days_in_order() {
        let mut existing = empty_week();
        existing[0].1 = "Monday".into();
        existing[2].1 = "Wednesday".into();
        existing[3].1 = " \n\t".into();
        assert_eq!(days_to_include(&existing, date(1)), vec![date(5), date(7)]);
    }

    #[test]
    fn includes_today_inside_week() {
        assert_eq!(days_to_include(&empty_week(), date(8)), vec![date(8)]);
    }

    #[test]
    fn excludes_today_outside_week() {
        assert!(days_to_include(&empty_week(), date(12)).is_empty());
        assert_eq!(week_days(date(5), date(12)), (5..=11).map(date).collect::<Vec<_>>());
    }
}
