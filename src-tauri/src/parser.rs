//! 同步仓 markdown 解析与字节保真编辑。
//!
//! 设计原则（PRD「文件层最小侵入」）：
//! - `lines` 是唯一事实源；`tasks` 是派生视图，每次手术后单行重解析保持一致。
//! - 所有编辑只动目标 token 的字节区间，其余字节原样保留。
//! - 往返保证：parse → serialize 与原文件字节一致（测试 G20）。
//!
//! 条目语法（与仓库 CLAUDE.md 一字不差）：
//! ```text
//! - [ ] (P1) [工作] 任务内容 #blocked 原因 #overdue #doing
//! ```
//! - 分类标签：封闭词表 `[工作]`/`[个人]`，仅识别「checkbox+优先级前缀之后紧跟」的位置。
//! - `#blocked` 后跟原因词（直到下一个 ` #` 或行尾）；`#overdue`/`#doing` 独立；
//!   标签之后的非 `#` 文本为人工注记（如 `#overdue 补记9/2`），原样保留。

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Category {
    Work,
    Personal,
    Uncategorized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Priority {
    P0,
    P1,
    P2,
    P3,
}

impl Priority {
    pub fn as_str(self) -> &'static str {
        match self {
            Priority::P0 => "P0",
            Priority::P1 => "P1",
            Priority::P2 => "P2",
            Priority::P3 => "P3",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Flag {
    Overdue,
    Doing,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskView {
    pub line_idx: usize,
    pub checked: bool,
    pub priority: Option<Priority>,
    pub category: Category,
    /// 展示文本（内容 + 注记），不含状态标签
    pub display: String,
    /// 纯内容段（编辑时替换的部分）
    pub content: String,
    pub doing: bool,
    pub overdue: bool,
    pub blocked_reason: Option<String>,
    /// 归属子行（缩进续行，如「老板要求: ...」）
    pub sub_lines: Vec<String>,
    /// 所在二级段（如「日任务」）
    pub section: String,
    /// 所在三级子区标题（如「⭐ 睡前必须完成」），无则 None
    pub subsection: Option<String>,
}

/// 一个任务行内部各 token 的字节区间（基于该行去掉行尾符的字符串）。
#[derive(Debug, Clone)]
struct Spans {
    cb: (usize, usize),                    // checkbox 内部字符
    prio: Option<(usize, usize)>,          // 含圆括号 (P1)
    cat: Option<(usize, usize)>,           // 含方括号 [工作]
    content: (usize, usize),               // 内容主体
    /// 已知状态标签，按出现顺序
    tags: Vec<TagSpan>,
}

#[derive(Debug, Clone)]
struct TagSpan {
    kind: FlagOrBlocked,
    span: (usize, usize),          // 标签本身（不含前导空格）
    reason: Option<(usize, usize)>, // #blocked 的原因词区间
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum FlagOrBlocked {
    Overdue,
    Doing,
    Blocked,
}

#[derive(Debug)]
struct ParsedLine {
    checked: bool,
    priority: Option<Priority>,
    category: Category,
    content: String,
    display: String,
    doing: bool,
    overdue: bool,
    blocked_reason: Option<String>,
    spans: Spans,
}

#[derive(Debug, Default)]
pub struct TodoFile {
    /// 去掉行尾符的每一行（唯一事实源）
    pub lines: Vec<String>,
    crlf: bool,
    trailing_newline: bool,
    /// 任务索引：tasks[i].line_idx 升序
    pub tasks: Vec<TaskView>,
    /// 每个任务的解析缓存，与 tasks 同下标
    parsed: Vec<ParsedLine>,
}

fn is_task_start(line: &str) -> bool {
    let b = line.as_bytes();
    b.len() >= 6 && b.starts_with(b"- [") && (b[3] == b' ' || b[3] == b'x') && b[4] == b']'
        && (b.len() == 5 || b[5] == b' ' || b[5] == 0x0)
}

fn is_heading(line: &str) -> Option<u8> {
    let t = line.trim_start();
    if t.starts_with("### ") {
        Some(3)
    } else if t.starts_with("## ") {
        Some(2)
    } else if t.starts_with("# ") {
        Some(1)
    } else {
        None
    }
}

/// 解析单个任务行。返回 None 表示不是任务行。
fn parse_task_line(line: &str) -> Option<ParsedLine> {
    if !is_task_start(line) {
        return None;
    }
    let b = line.as_bytes();
    let cb_span = (3usize, 4usize);
    let checked = b[3] == b'x';
    let mut pos = 5usize; // "] " 之后
    if pos < b.len() && b[pos] == b' ' {
        pos += 1;
    }

    // 优先级 (P0..P3)
    let mut priority = None;
    let mut prio_span = None;
    if b.len() >= pos + 4 && b[pos] == b'(' && b[pos + 1] == b'P' {
        let d = b[pos + 2];
        if (b'0'..=b'3').contains(&d) && b[pos + 3] == b')' {
            priority = Some(match d {
                b'0' => Priority::P0,
                b'1' => Priority::P1,
                b'2' => Priority::P2,
                _ => Priority::P3,
            });
            prio_span = Some((pos, pos + 4));
            pos += 4;
            if pos < b.len() && b[pos] == b' ' {
                pos += 1;
            }
        }
    }

    // 分类标签：封闭词表，紧跟前缀
    let mut cat_span = None;
    let mut category = Category::Uncategorized;
    for (word, cat) in [("[工作]", Category::Work), ("[个人]", Category::Personal)] {
        let wb = word.as_bytes();
        if b.len() >= pos + wb.len() && &b[pos..pos + wb.len()] == wb {
            cat_span = Some((pos, pos + wb.len()));
            category = cat;
            pos += wb.len();
            if pos < b.len() && b[pos] == b' ' {
                pos += 1;
            }
            break;
        }
    }

    // 内容区 + 尾部标签扫描
    let content_start = pos;
    let mut tags: Vec<TagSpan> = Vec::new();
    let mut content_end = b.len();
    let mut i = pos;
    let mut scanning_tags = false;
    while i < b.len() {
        if b[i] == b'#' && (i == 0 || b[i - 1] == b' ') {
            // 潜在标签：识别封闭词表
            let rest = &line[i + 1..];
            let (kind, len) = if rest.starts_with("overdue") {
                (Some(FlagOrBlocked::Overdue), "overdue".len())
            } else if rest.starts_with("doing") {
                (Some(FlagOrBlocked::Doing), "doing".len())
            } else if rest.starts_with("blocked") {
                (Some(FlagOrBlocked::Blocked), "blocked".len())
            } else {
                (None, 0)
            };
            if let Some(kind) = kind {
                if !scanning_tags {
                    content_end = trim_end_at(line, i);
                    scanning_tags = true;
                }
                let tag_start = i;
                let tag_end = i + 1 + len;
                let mut reason = None;
                let mut j = tag_end;
                if kind == FlagOrBlocked::Blocked {
                    // 原因词：直到下一个 " #" 或行尾
                    let rs = skip_spaces(line, j);
                    let mut re = rs;
                    while re < b.len() {
                        if b[re] == b'#' && re > rs && b[re - 1] == b' ' {
                            break;
                        }
                        re += 1;
                    }
                    let re_trim = trim_end_at(line, re);
                    if re_trim > rs {
                        reason = Some((rs, re_trim));
                        j = re_trim;
                    }
                }
                tags.push(TagSpan { kind, span: (tag_start, tag_end), reason });
                i = j;
                continue;
            }
        }
        i += 1;
    }

    let content_end = if scanning_tags { content_end } else { b.len() };
    // 前缀解析吃掉的分隔空格可能再被 trim_end_at 吃一遍（如 "- [ ] (P1) #doing"），
    // 起点越过终点的反向切片会 panic——release 是 panic=abort，整进程死。
    let content_start = content_start.min(content_end);
    let content = line[content_start..content_end].trim().to_string();

    // 注记：标签之后的非标签文本（如 "补记9/2"）
    let mut annotations: Vec<&str> = Vec::new();
    let mut cur = content_end;
    for t in &tags {
        let seg_end = t.span.0;
        if cur < seg_end {
            let seg = line[cur..seg_end].trim();
            if !seg.is_empty() {
                annotations.push(seg);
            }
        }
        cur = t.reason.map(|r| r.1).unwrap_or(t.span.1);
    }
    if cur < b.len() {
        let seg = line[cur..].trim();
        if !seg.is_empty() {
            annotations.push(seg);
        }
    }

    let doing = tags.iter().any(|t| t.kind == FlagOrBlocked::Doing);
    let overdue = tags.iter().any(|t| t.kind == FlagOrBlocked::Overdue);
    let blocked_reason = tags
        .iter()
        .find(|t| t.kind == FlagOrBlocked::Blocked)
        .and_then(|t| t.reason.map(|(s, e)| line[s..e].to_string()));

    let mut display = content.clone();
    for a in annotations {
        display.push(' ');
        display.push_str(a);
    }

    Some(ParsedLine {
        checked,
        priority,
        category,
        content,
        display,
        doing,
        overdue,
        blocked_reason,
        spans: Spans { cb: cb_span, prio: prio_span, cat: cat_span, content: (content_start, content_end), tags },
    })
}

fn skip_spaces(line: &str, mut i: usize) -> usize {
    let b = line.as_bytes();
    while i < b.len() && b[i] == b' ' {
        i += 1;
    }
    i
}

/// 把 end 收敛到不包含尾随空格的位置（内容与标签间的分隔空格不属于内容）。
fn trim_end_at(line: &str, end: usize) -> usize {
    let b = line.as_bytes();
    let mut e = end.min(b.len());
    while e > 0 && b[e - 1] == b' ' {
        e -= 1;
    }
    e
}

/// 用户自由文本入库守卫（命令层调用）：
/// - 拒绝换行：markdown 按行组织，注入 \n 会伪造任务行/标题行；
/// - 拒绝活标签词：与 parse_line 相同的识别规则（行首/空格后跟 #overdue/#doing/#blocked），
///   否则用户文本下次解析变成真状态标签，#overdue 还无法从 UI 移除。
pub fn validate_user_text(text: &str) -> Result<(), String> {
    if text.contains(['\n', '\r']) {
        return Err("内容不能包含换行".into());
    }
    let b = text.as_bytes();
    for i in 0..b.len() {
        if b[i] == b'#' && (i == 0 || b[i - 1] == b' ') {
            let rest = &text[i + 1..];
            if ["overdue", "doing", "blocked"].iter().any(|w| rest.starts_with(w)) {
                return Err(
                    "内容不能包含 #overdue/#doing/#blocked 状态标签（状态用右键菜单管理）".into(),
                );
            }
        }
    }
    Ok(())
}

/// 撤销恢复的原始行：只拦换行（行内标签合法——本就是文件里的真实任务行）。
pub fn validate_restore_lines(lines: &[String]) -> Result<(), String> {
    if lines.iter().any(|l| l.contains(['\n', '\r'])) {
        return Err("恢复的行不能包含换行".into());
    }
    Ok(())
}

impl TodoFile {
    pub fn parse(text: &str) -> TodoFile {
        let crlf = text.contains("\r\n");
        let trailing_newline = text.is_empty() || text.ends_with('\n');
        let normalized = if crlf { text.replace("\r\n", "\n") } else { text.to_string() };
        let mut lines: Vec<String> = normalized.split('\n').map(|s| s.to_string()).collect();
        // "a\n" split => ["a", ""]：末尾空串代表换行符，收回
        if trailing_newline && lines.last().is_some_and(|s| s.is_empty()) {
            lines.pop();
        }

        let mut tasks: Vec<TaskView> = Vec::new();
        let mut parsed: Vec<ParsedLine> = Vec::new();
        let mut section = String::new();
        let mut subsection: Option<String> = None;
        let mut last_task: Option<usize> = None; // lines 下标

        for (idx, line) in lines.iter().enumerate() {
            if let Some(level) = is_heading(line) {
                let title = line.trim_start().trim_start_matches('#').trim().to_string();
                if level <= 2 {
                    section = title;
                    subsection = None;
                } else {
                    subsection = Some(title);
                }
                last_task = None;
                continue;
            }
            if let Some(p) = parse_task_line(line) {
                let view = TaskView {
                    line_idx: idx,
                    checked: p.checked,
                    priority: p.priority,
                    category: p.category,
                    display: p.display.clone(),
                    content: p.content.clone(),
                    doing: p.doing,
                    overdue: p.overdue,
                    blocked_reason: p.blocked_reason.clone(),
                    sub_lines: Vec::new(),
                    section: section.clone(),
                    subsection: subsection.clone(),
                };
                tasks.push(view);
                parsed.push(p);
                last_task = Some(idx);
            } else {
                // 缩进续行归属上一个任务
                let is_indented = line.starts_with(' ') || line.starts_with('\t');
                if is_indented && !line.trim().is_empty() {
                    if let Some(t) = last_task {
                        let view = tasks.last_mut().unwrap();
                        if view.line_idx == t {
                            view.sub_lines.push(line.trim_start().to_string());
                        }
                    }
                }
            }
        }

        TodoFile { lines, crlf, trailing_newline, tasks, parsed }
    }

    /// 序列化。未做任何编辑时与原文字节一致。
    pub fn serialize(&self) -> String {
        let eol = if self.crlf { "\r\n" } else { "\n" };
        let mut out = self.lines.join(eol);
        if self.trailing_newline {
            out.push_str(eol);
        }
        out
    }

    fn task_by_line(&self, line_idx: usize) -> Option<usize> {
        self.tasks.iter().position(|t| t.line_idx == line_idx)
    }

    /// 手术后：用新行重解析该任务，同步派生视图与缓存。
    fn refresh(&mut self, slot: usize) {
        let new_line = self.lines[self.tasks[slot].line_idx].clone();
        match parse_task_line(&new_line) {
            Some(p) => {
                let v = &mut self.tasks[slot];
                v.checked = p.checked;
                v.priority = p.priority;
                v.category = p.category;
                v.display = p.display.clone();
                v.content = p.content.clone();
                v.doing = p.doing;
                v.overdue = p.overdue;
                v.blocked_reason = p.blocked_reason.clone();
                self.parsed[slot] = p;
            }
            None => { /* 行不再是任务：保守保留旧视图，调用方不应制造这种编辑 */ }
        }
    }

    /// 勾选翻转。只改 [ ] ↔ [x] 的一个字符。
    pub fn set_checked(&mut self, line_idx: usize, checked: bool) -> Result<(), String> {
        let slot = self.task_by_line(line_idx).ok_or("line 不是任务行")?;
        let (s, e) = self.parsed[slot].spans.cb;
        let line = self.lines[line_idx].clone();
        let ch = if checked { "x" } else { " " };
        let bytes = line.as_bytes();
        if s >= e || e > bytes.len() {
            return Err("checkbox 区间非法".into());
        }
        let mut new_line = String::with_capacity(line.len());
        new_line.push_str(&line[..s]);
        new_line.push_str(ch);
        new_line.push_str(&line[e..]);
        self.lines[line_idx] = new_line;
        self.refresh(slot);
        Ok(())
    }

    /// 设置分类标签：Some=插入/替换 `[工作]`/`[个人]`，None=移除。
    pub fn set_category(&mut self, line_idx: usize, cat: Option<Category>) -> Result<(), String> {
        let slot = self.task_by_line(line_idx).ok_or("line 不是任务行")?;
        let old = self.parsed[slot].spans.cat;
        let line = self.lines[line_idx].clone();
        let mut new_line = String::with_capacity(line.len() + 8);
        match (old, cat) {
            (None, None) => {}
            (Some((s, e)), None) => {
                // 移除标签 + 其后一个空格（若有）
                let mut cut = e;
                let b = line.as_bytes();
                if cut < b.len() && b[cut] == b' ' {
                    cut += 1;
                }
                new_line.push_str(&line[..s]);
                new_line.push_str(&line[cut..]);
            }
            (None, Some(c)) => {
                // 插入位置：内容区起点（即优先级/checkbox 前缀之后）
                let anchor = self.parsed[slot].spans.content.0;
                let word = match c {
                    Category::Work => "[工作] ",
                    _ => "[个人] ",
                };
                new_line.push_str(&line[..anchor]);
                new_line.push_str(word);
                new_line.push_str(&line[anchor..]);
            }
            (Some((s, e)), Some(c)) => {
                let word = match c {
                    Category::Work => "[工作]",
                    _ => "[个人]",
                };
                new_line.push_str(&line[..s]);
                new_line.push_str(word);
                new_line.push_str(&line[e..]);
            }
        }
        if new_line != line {
            self.lines[line_idx] = new_line;
            self.refresh(slot);
        }
        Ok(())
    }

    /// 设置优先级：Some=插入/替换 `(P1)`，None=移除。优先级位于 checkbox 后、分类前。
    pub fn set_priority(&mut self, line_idx: usize, prio: Option<Priority>) -> Result<(), String> {
        let slot = self.task_by_line(line_idx).ok_or("line 不是任务行")?;
        let old = self.parsed[slot].spans.prio;
        let line = self.lines[line_idx].clone();
        let mut new_line = String::with_capacity(line.len() + 5);
        match (old, prio) {
            (None, None) => {}
            (Some((s, e)), None) => {
                // 移除优先级 + 其后一个空格（若有）
                let mut cut = e;
                let b = line.as_bytes();
                if cut < b.len() && b[cut] == b' ' {
                    cut += 1;
                }
                new_line.push_str(&line[..s]);
                new_line.push_str(&line[cut..]);
            }
            (None, Some(p)) => {
                // 插入位置：分类标签或内容区的起点（优先级永远在最前缀）
                let anchor = self.parsed[slot]
                    .spans
                    .cat
                    .map(|(s, _)| s)
                    .unwrap_or(self.parsed[slot].spans.content.0);
                new_line.push_str(&line[..anchor]);
                new_line.push('(');
                new_line.push_str(p.as_str());
                new_line.push_str(") ");
                new_line.push_str(&line[anchor..]);
            }
            (Some((s, e)), Some(p)) => {
                // 原地替换 `(P0)` ↔ `(P2)`
                new_line.push_str(&line[..s]);
                new_line.push('(');
                new_line.push_str(p.as_str());
                new_line.push(')');
                new_line.push_str(&line[e..]);
            }
        }
        if new_line != line {
            self.lines[line_idx] = new_line;
            self.refresh(slot);
        }
        Ok(())
    }

    /// 追加状态标签（#overdue / #doing）。
    pub fn add_flag(&mut self, line_idx: usize, flag: Flag) -> Result<(), String> {
        let slot = self.task_by_line(line_idx).ok_or("line 不是任务行")?;
        if match flag {
            Flag::Overdue => self.parsed[slot].overdue,
            Flag::Doing => self.parsed[slot].doing,
        } {
            return Ok(()); // 幂等
        }
        let word = match flag {
            Flag::Overdue => " #overdue",
            Flag::Doing => " #doing",
        };
        let mut new_line = self.lines[line_idx].clone();
        new_line.push_str(word);
        self.lines[line_idx] = new_line;
        self.refresh(slot);
        Ok(())
    }

    /// 移除状态标签。#blocked 连带原因词一起移除（原因属于标签）。
    pub fn remove_flag(&mut self, line_idx: usize, flag: Flag) -> Result<(), String> {
        let slot = self.task_by_line(line_idx).ok_or("line 不是任务行")?;
        let tag = self.parsed[slot].spans.tags.iter().find(|t| match flag {
            Flag::Overdue => t.kind == FlagOrBlocked::Overdue,
            Flag::Doing => t.kind == FlagOrBlocked::Doing,
        });
        let Some(tag) = tag else { return Ok(()) };
        let (s, e) = tag.span;
        let line = self.lines[line_idx].clone();
        let b = line.as_bytes();
        let mut start = s;
        if start > 0 && b[start - 1] == b' ' {
            start -= 1;
        }
        let mut new_line = String::with_capacity(line.len());
        new_line.push_str(&line[..start]);
        new_line.push_str(&line[e..]);
        self.lines[line_idx] = new_line;
        self.refresh(slot);
        Ok(())
    }

    /// 替换内容主体（保留前缀、标签、注记）。
    pub fn set_content(&mut self, line_idx: usize, new_content: &str) -> Result<(), String> {
        let slot = self.task_by_line(line_idx).ok_or("line 不是任务行")?;
        let (s, e) = self.parsed[slot].spans.content;
        let line = self.lines[line_idx].clone();
        let mut new_line = String::with_capacity(line.len());
        new_line.push_str(&line[..s]);
        new_line.push_str(new_content);
        new_line.push_str(&line[e..]);
        self.lines[line_idx] = new_line;
        self.refresh(slot);
        Ok(())
    }

    /// 设置/清除 `#blocked`（PRD F7：blocked 可写且带原因文本）。
    /// Some → 行尾写 ` #blocked 原因`（已有 blocked 先移除再写，原因更新）；
    /// None → 移除标签连带原因词（原因属于标签）。
    pub fn set_blocked(&mut self, line_idx: usize, reason: Option<&str>) -> Result<(), String> {
        let slot = self.task_by_line(line_idx).ok_or("line 不是任务行")?;
        let line = self.lines[line_idx].clone();
        let b = line.as_bytes();

        // 移除既有 #blocked（含前导空格与原因词；原因词区间解析时已收口）
        let mut new_line = line.clone();
        if let Some(tag) = self.parsed[slot].spans.tags.iter().find(|t| t.kind == FlagOrBlocked::Blocked) {
            let mut end = tag.span.1;
            if let Some((_, re)) = tag.reason {
                end = end.max(re);
            }
            let mut start = tag.span.0;
            if start > 0 && b[start - 1] == b' ' {
                start -= 1;
            }
            new_line = format!("{}{}", &line[..start], &line[end..]);
        }

        if let Some(r) = reason {
            let r = r.trim();
            if r.is_empty() {
                return Err("blocked 原因不能为空白".into());
            }
            new_line.push_str(&format!(" #blocked {r}"));
        }
        self.lines[line_idx] = new_line;
        // 重建该行解析缓存（行内区间全部变化）
        let text = self.serialize();
        *self = TodoFile::parse(&text);
        Ok(())
    }

    /// 删除任务行（连同其缩进子行），返回被删原始行供撤销恢复。
    pub fn delete_task(&mut self, line_idx: usize) -> Result<Vec<String>, String> {
        let _slot = self.task_by_line(line_idx).ok_or("line 不是任务行")?;
        let mut end = line_idx + 1;
        while end < self.lines.len() {
            let l = &self.lines[end];
            let indented = l.starts_with(' ') || l.starts_with('\t');
            if indented && !l.trim().is_empty() {
                end += 1;
            } else {
                break;
            }
        }
        let removed: Vec<String> = self.lines.drain(line_idx..end).collect();
        // 重建派生视图（行号整体变化）
        let text = self.serialize();
        *self = TodoFile::parse(&text);
        Ok(removed)
    }

    /// 在 line_idx 处原样插回若干行（撤销删除；行内容不做任何改写）。
    pub fn insert_lines_at(&mut self, line_idx: usize, removed: Vec<String>) {
        let at = line_idx.min(self.lines.len());
        for (i, l) in removed.into_iter().enumerate() {
            self.lines.insert(at + i, l);
        }
        let text = self.serialize();
        *self = TodoFile::parse(&text);
    }

    /// 定位指定二级段「平铺任务区」末尾的插入行号。
    /// 平铺区 = 段落标题（含紧随的注释行）之后、第一个三级子区/下一二级段/EOF 之前；
    /// 插入点 = 区内最后一个非空行之后。add_task 与跨日流转的整块搬运共用。
    pub fn section_flat_insert_at(&self, section: &str) -> Result<usize, String> {
        let sec_lower = section.to_lowercase();
        let mut sec_range: Option<(usize, usize)> = None;
        let mut i = 0usize;
        let mut cur: Option<(usize, usize)> = None;
        while i < self.lines.len() {
            if let Some(level) = is_heading(&self.lines[i]) {
                if level <= 2 {
                    // 收口：段终点 = 当前标题行（占位值 lines.len() 作废）
                    if let Some((s, _)) = cur.take() {
                        sec_range = Some((s, i));
                    }
                    let title = self.lines[i].trim_start().trim_start_matches('#').trim().to_string();
                    if title.to_lowercase() == sec_lower {
                        cur = Some((i + 1, self.lines.len()));
                    }
                } else if cur.is_some() {
                    let (s, _) = cur.unwrap();
                    cur = Some((s, i));
                    // 继续找段尾由下一个 ## 或 EOF 决定；这里直接封口
                    sec_range = cur;
                    cur = None;
                    i += 1;
                    continue;
                }
            }
            i += 1;
        }
        if let Some(r) = cur {
            sec_range = Some(r);
        }
        let (sec_start, sec_end) = sec_range.ok_or(format!("未找到段落 ## {}", section))?;

        // 在平铺区内找最后一个非空行
        let mut insert_at = sec_start;
        let mut j = sec_start;
        while j < sec_end {
            if !self.lines[j].trim().is_empty() {
                insert_at = j + 1;
            }
            j += 1;
        }
        Ok(insert_at)
    }

    /// 在指定二级段的「平铺任务区」末尾追加新任务。
    pub fn add_task(
        &mut self,
        section: &str,
        category: Category,
        priority: Priority,
        text: &str,
    ) -> Result<usize, String> {
        let insert_at = self.section_flat_insert_at(section)?;

        let cat_word = match category {
            Category::Work => " [工作] ",
            Category::Personal => " [个人] ",
            Category::Uncategorized => " ",
        };
        let line = format!("- [ ] ({}){}{}", priority.as_str(), cat_word, text);
        self.lines.insert(insert_at, line);
        let text = self.serialize();
        *self = TodoFile::parse(&text);
        Ok(insert_at)
    }
}
