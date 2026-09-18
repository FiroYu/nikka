//! 跨日自动流转（Win 专属，2026-09-16 需求）：历史日文件里未勾选的日任务
//! 自动搬入今天，搬走后旧日只剩已完成项；勾掉即永不再出现。
//!
//! - 范围：全部笔记本 × 全部早于今天的 `days/YYYY-MM-DD.md`，仅 `## 日任务`
//!   段的未勾选任务。周任务、速记、其他章节、未来日期一概不动。
//! - 字段：原始行块整块搬运（优先级/分类/#doing/#blocked 及原因/人工注记/
//!   缩进子行原样保留），仅目的任务剥掉 `#overdue`（D5：旧日期的状态不带走）。
//!
//! 崩溃收敛设计（不设事务日志）：整轮持 io 锁，先写今天的目标文件、再写各
//! 源文件。中途崩溃留下的中间态（任务同时存在于源与今天）由下轮的「同文
//! 未勾选去重」收口——搬运前若今天已存在同内容未勾选任务，只删源不重复加，
//! 状态单调收敛。同文去重同样作用于正常轮：用户手动在今天重写过同文案的
//! 未完成事项时，历史副本并入今天的那条，不重复展示。
//!
//! 多机边界：Git 不提供任务级恰好一次。本期以单台 Windows 作为自动执行端
//! （`STICKY_AUTO_ROLLOVER=0` 可整机关闭），其余设备经同步接收流转结果；
//! 安卓保持 F14 手动复制不动。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;

use crate::notebook;
use crate::parser::{Flag, TodoFile};
use crate::repo::{DAY_TASK_SECTION, FileKind};
use crate::store::{CmdError, StickyStore, SyncUiState};

#[derive(Debug, Default, PartialEq)]
pub struct CarryReport {
    /// 搬运的任务数
    pub moved: usize,
    /// 因今天已有同文未勾选任务而只删源不重复加的数量
    pub merged: usize,
}

/// 自动流转总开关：`STICKY_AUTO_ROLLOVER=0` 关闭（多 Windows 部署时，
/// 非执行端的机器显式关闭，避免离线双端各自搬运）。
pub fn enabled() -> bool {
    std::env::var("STICKY_AUTO_ROLLOVER").map(|v| v != "0").unwrap_or(true)
}

/// 执行一轮流转。整轮持 io 锁（与编辑、git 同步互斥）；锁内纯本地文件操作。
pub fn carry_over(store: &StickyStore, today: NaiveDate) -> Result<CarryReport, CmdError> {
    if !enabled() {
        return Ok(CarryReport::default());
    }
    store.ensure_repo_ready()?;
    let _g = store.io_lock();
    // 冲突未解时工作区可能有冲突标记，解析会产出垃圾任务——跳过本轮，
    // 等人工处理完冲突后由下个同步周期补扫。
    if matches!(store.status(), SyncUiState::Conflict { .. }) {
        return Ok(CarryReport::default());
    }
    let repo = store.engine().repo_dir();
    let mut ids = vec![notebook::DEFAULT_NOTEBOOK.to_string()];
    for book in notebook::list(repo)? {
        ids.push(book.id);
    }
    let mut report = CarryReport::default();
    for id in &ids {
        let r = carry_notebook(store, id, today)?;
        report.moved += r.moved;
        report.merged += r.merged;
    }
    if report.moved + report.merged > 0 {
        store.add_pending_edits(report.moved + report.merged);
    }
    Ok(report)
}

/// 枚举某笔记本 days/ 下早于 today 的日期，升序。文件名不合约定的一律跳过。
fn past_day_dates(days_dir: &Path, today: NaiveDate) -> Vec<NaiveDate> {
    let mut dates = Vec::new();
    let Ok(entries) = std::fs::read_dir(days_dir) else { return dates };
    for entry in entries.flatten() {
        // OsString 先落地再借（let-else 链里临时值活不过语句尾）
        let name = entry.file_name();
        let Some(stem) = name.to_str().and_then(|n| n.strip_suffix(".md")) else { continue };
        let Ok(date) = NaiveDate::parse_from_str(stem, "%Y-%m-%d") else { continue };
        if date < today {
            dates.push(date);
        }
    }
    dates.sort_unstable();
    dates.dedup();
    dates
}

struct PendingSource {
    path: PathBuf,
    text: String,
}

fn carry_notebook(store: &StickyStore, notebook_id: &str, today: NaiveDate) -> Result<CarryReport, CmdError> {
    let root = notebook::root(store.engine().repo_dir(), notebook_id)?;
    let dates = past_day_dates(&root.join("days"), today);
    if dates.is_empty() {
        return Ok(CarryReport::default());
    }

    // 今天的目标文件（内存态）。缺失时先留空，首笔搬运前套模板（PRD G5：
    // 不因扫描而建文件）；无搬运则整轮零写入。
    let today_kind = FileKind::Day(today);
    let today_path = today_kind.abs_path(&root);
    let today_text = read_optional(&today_path, &today_kind.rel_path())?;
    let mut target: Option<TodoFile> = today_text.as_deref().map(TodoFile::parse);
    // 去重集合：今天已有的未勾选日任务内容（同文未勾选 = 并入，不重复加）
    let mut seen: HashSet<String> = target.as_ref()
        .map(|t| t.tasks.iter()
            .filter(|v| !v.checked && v.section == DAY_TASK_SECTION)
            .map(|v| v.content.clone()).collect())
        .unwrap_or_default();

    let mut sources: Vec<PendingSource> = Vec::new();
    let mut report = CarryReport::default();

    for date in dates {
        let kind = FileKind::Day(date);
        let path = kind.abs_path(&root);
        let Some(text) = read_optional(&path, &kind.rel_path())? else { continue };
        let mut file = TodoFile::parse(&text);
        let candidates: Vec<usize> = file.tasks.iter()
            .filter(|t| !t.checked && t.section == DAY_TASK_SECTION)
            .map(|t| t.line_idx)
            .collect();
        if candidates.is_empty() {
            continue;
        }
        let mut moved_here = 0usize;
        let mut merged_here = 0usize;
        // 降序删除避免行号漂移；块按当日行序进入目标末尾
        for line_idx in candidates.into_iter().rev() {
            let content = file.tasks.iter()
                .find(|t| t.line_idx == line_idx)
                .map(|t| t.content.clone());
            let _ = file.remove_flag(line_idx, Flag::Overdue); // 幂等；旧日期状态不带走
            let block = match file.delete_task(line_idx) {
                Ok(b) => b,
                Err(e) => return Err(CmdError::bad_request(format!("{} 第 {line_idx} 行搬运失败: {e}", kind.rel_path()))),
            };
            match content.filter(|c| !seen.insert(c.clone())) {
                Some(_) => merged_here += 1, // 今天已有同文未勾选：只删源
                None => {
                    if target.is_none() {
                        let tpl = store.materialize_template(&today_kind)?;
                        target = Some(TodoFile::parse(&tpl));
                    }
                    let target = target.as_mut().expect("刚在上面置位");
                    let at = target.section_flat_insert_at(DAY_TASK_SECTION).map_err(CmdError::bad_request)?;
                    target.insert_lines_at(at, block);
                    moved_here += 1;
                }
            }
        }
        if moved_here + merged_here > 0 {
            sources.push(PendingSource { path, text: file.serialize() });
            report.moved += moved_here;
            report.merged += merged_here;
        }
    }

    if report.moved == 0 && report.merged == 0 {
        return Ok(report);
    }
    // 先写目标再写源（崩溃收敛顺序，见模块注释）。纯合并轮目标无变化不写。
    if report.moved > 0 {
        let target = target.expect("有搬运必有目标（缺文件时已套模板）");
        write_file(&today_path, &target.serialize())?;
    }
    for src in &sources {
        write_file(&src.path, &src.text)?;
    }
    Ok(report)
}

fn read_optional(path: &Path, rel: &str) -> Result<Option<String>, CmdError> {
    match std::fs::read_to_string(path) {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CmdError::internal(format!("读取 {rel} 失败: {e}"))),
    }
}

fn write_file(path: &Path, text: &str) -> Result<(), CmdError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| CmdError::internal(format!("建目录失败: {e}")))?;
    }
    notebook::atomic_write(path, text)
}
