//! 同步仓的文件路径约定与模板套用。
//!
//! 路径契约（与仓库 CLAUDE.md 一字不差）：
//! - `days/YYYY-MM-DD.md`
//! - `weeks/YYYY-Www.md`（ISO 周，周一起始，ww 两位零填充）
//! - 缺文件时套 `_templates/day.md` / `_templates/week.md`
//!
//! 模板占位符（真实模板文件实测）：
//! - day：`{{YYYY-MM-DD}}`、`{{WEEKDAY_CN}}`
//! - week：`{{YYYY}}`、`{{WW}}`、`{{START}}`、`{{END}}`（均为 YYYY-MM-DD）

use chrono::{Datelike, Days, IsoWeek, NaiveDate, Weekday};
use std::fmt;
use std::path::PathBuf;

/// 新任务追加的段落标题（与模板实测一致）。
pub const DAY_TASK_SECTION: &str = "日任务";
pub const WEEK_TASK_SECTION: &str = "本周任务";

/// 一类待办文件（日或周）。v1 不做月/项目文件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Day(NaiveDate),
    Week(NaiveDate),
}

impl FileKind {
    /// 相对仓库根的路径（正斜杠，与仓库约定一致；拼接本地根目录用 `abs_path`）。
    pub fn rel_path(&self) -> String {
        match self {
            FileKind::Day(d) => format!("days/{}.md", fmt_date(*d)),
            FileKind::Week(d) => format!("weeks/{}.md", fmt_week(iso_of(*d))),
        }
    }

    /// 仓库本地克隆中的绝对路径（正斜杠在 Windows API 中同样有效）。
    pub fn abs_path(&self, repo_root: &std::path::Path) -> PathBuf {
        repo_root.join(self.rel_path())
    }

    /// 对应模板文件的仓库相对路径。
    pub fn template_rel(&self) -> &'static str {
        match self {
            FileKind::Day(_) => "_templates/day.md",
            FileKind::Week(_) => "_templates/week.md",
        }
    }

    /// 套用模板占位符，产出可写入的文件内容。
    pub fn fill_template(&self, template: &str) -> String {
        match self {
            FileKind::Day(d) => template
                .replace("{{YYYY-MM-DD}}", &fmt_date(*d))
                .replace("{{WEEKDAY_CN}}", weekday_cn(d.weekday())),
            FileKind::Week(d) => {
                let iso = iso_of(*d);
                let monday = monday_of(iso);
                let sunday = monday.checked_add_days(Days::new(6)).expect("合法日期 +6 天");
                template
                    .replace("{{YYYY}}", &iso.year().to_string())
                    .replace("{{WW}}", &format!("{:02}", iso.week()))
                    .replace("{{START}}", &fmt_date(monday))
                    .replace("{{END}}", &fmt_date(sunday))
            }
        }
    }
}

fn iso_of(d: NaiveDate) -> IsoWeek {
    d.iso_week()
}

fn monday_of(iso: IsoWeek) -> NaiveDate {
    NaiveDate::from_isoywd_opt(iso.year(), iso.week(), Weekday::Mon)
        .expect("ISO 年周必然对应一个周一")
}

pub fn fmt_date(d: NaiveDate) -> String {
    format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day())
}

fn fmt_week(iso: IsoWeek) -> String {
    format!("{}-W{:02}", iso.year(), iso.week())
}

/// 模板占位符 {{WEEKDAY_CN}} 的值：仅"一"~"日"（模板自带 `周` 前缀，实测 `_templates/day.md`）。
fn weekday_cn(w: Weekday) -> &'static str {
    match w {
        Weekday::Mon => "一",
        Weekday::Tue => "二",
        Weekday::Wed => "三",
        Weekday::Thu => "四",
        Weekday::Fri => "五",
        Weekday::Sat => "六",
        Weekday::Sun => "日",
    }
}

impl fmt::Display for FileKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.rel_path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn day_and_week_paths() {
        assert_eq!(FileKind::Day(d("2026-09-10")).rel_path(), "days/2026-09-10.md");
        assert_eq!(FileKind::Week(d("2026-09-10")).rel_path(), "weeks/2026-W37.md");
    }

    #[test]
    fn iso_week_boundaries() {
        // 2026-01-01 是周四 → 2026-W01
        assert_eq!(FileKind::Week(d("2026-01-01")).rel_path(), "weeks/2026-W01.md");
        // 2027-01-01 是周五 → 属于 2026-W53
        assert_eq!(FileKind::Week(d("2027-01-01")).rel_path(), "weeks/2026-W53.md");
        // 2025-12-29 是周一 → 属于 2026-W01（ISO 年 ≠ 公历年）
        assert_eq!(FileKind::Week(d("2025-12-29")).rel_path(), "weeks/2026-W01.md");
        // 年中跨周：2026-09-06 周日 → W36；09-07 周一 → W37
        assert_eq!(FileKind::Week(d("2026-09-06")).rel_path(), "weeks/2026-W36.md");
        assert_eq!(FileKind::Week(d("2026-09-07")).rel_path(), "weeks/2026-W37.md");
    }

    #[test]
    fn day_template_fill() {
        let tpl = "# {{YYYY-MM-DD}} (周{{WEEKDAY_CN}})\n\n> 本日重点: \n\n## 日任务\n";
        let out = FileKind::Day(d("2026-09-10")).fill_template(tpl);
        assert!(out.starts_with("# 2026-09-10 (周四)\n"), "实际: {out}");
        assert_eq!(out, "# 2026-09-10 (周四)\n\n> 本日重点: \n\n## 日任务\n");
    }

    #[test]
    fn week_template_fill() {
        let tpl = "# {{YYYY}}-W{{WW}} ({{START}} ~ {{END}})\n\n## 本周任务\n";
        let out = FileKind::Week(d("2026-09-10")).fill_template(tpl);
        assert_eq!(out, "# 2026-W37 (2026-09-07 ~ 2026-09-13)\n\n## 本周任务\n");
        // 跨年周：2027-01-01（周五）属于 2026-W53，周一在 2026-12-28
        let out2 = FileKind::Week(d("2027-01-01")).fill_template(tpl);
        assert!(out2.starts_with("# 2026-W53 (2026-12-28 ~ 2027-01-03)"), "实际: {out2}");
    }

    #[test]
    fn abs_path_joins() {
        let p = FileKind::Day(d("2026-09-10")).abs_path(std::path::Path::new("C:/repo"));
        assert_eq!(p, PathBuf::from("C:/repo/days/2026-09-10.md"));
    }
}
