//! 解析器测试：字节保真往返（G20）+ 手术最小侵入（G21）+ 两层结构解析。
//!
//! fixtures/ 下是样例文件（08-11/12/13 日文件、模板；结构与真实同步仓一致）。

use app_lib::parser::{Category, Flag, Priority, TodoFile};

fn fixture(rel: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("读 {path:?} 失败: {e}"))
}

/// G20：真实文件往返字节一致。
#[test]
fn roundtrip_real_files() {
    for rel in [
        "days/2026-08-11.md",
        "days/2026-08-12.md",
        "days/2026-08-13.md",
        "_templates/day.md",
        "_templates/week.md",
        "weeks/2026-W33.md",
        "weeks/2026-W34.md",
    ] {
        let text = fixture(rel);
        let tf = TodoFile::parse(&text);
        assert_eq!(tf.serialize(), text, "往返不一致: {rel}");
    }
}

/// G20 补充：CRLF 行尾与无尾换行文件的往返。
#[test]
fn roundtrip_eol_variants() {
    let crlf = "# 2026-08-12 (周三)\r\n\r\n## 日任务\r\n\r\n- [ ] (P1) 任务A #overdue\r\n- [x] (P2) 任务B\r\n";
    assert_eq!(TodoFile::parse(crlf).serialize(), crlf);

    let no_trailing = "- [ ] (P1) 无尾换行";
    assert_eq!(TodoFile::parse(no_trailing).serialize(), no_trailing);
}

/// 两层结构：## 段落 × ### 子区；备注里的普通 `- ` 列表不是任务。
#[test]
fn two_layer_structure() {
    let tf = TodoFile::parse(&fixture("days/2026-08-12.md"));
    assert_eq!(tf.tasks.len(), 10, "08-12 应有 10 个任务");

    let day = tf.tasks.iter().filter(|t| t.section == "日任务").collect::<Vec<_>>();
    assert_eq!(day.len(), 10, "全部在日任务段（含子区）");
    assert_eq!(day.iter().filter(|t| t.subsection.is_none()).count(), 5);
    assert_eq!(
        day.iter().filter(|t| t.subsection.as_deref() == Some("⭐ 睡前必须完成")).count(),
        5,
        "⭐ 子区 5 条"
    );
    // 完成事项/备注段没有任务（备注是普通 - 列表）
    assert!(tf.tasks.iter().all(|t| t.section != "备注"));
    assert!(tf.tasks.iter().all(|t| t.section != "完成事项"));
}

/// N3：完成尾注 ✅ 保留在 display 中，勾选翻转不动它。
#[test]
fn done_tail_notes_in_display() {
    let tf = TodoFile::parse(&fixture("days/2026-08-12.md"));
    let t = tf
        .tasks
        .iter()
        .find(|t| t.checked && t.content.contains("家庭相册"))
        .expect("找到已完成条目");
    assert!(t.display.contains("8/13完成"), "尾注进 display: {}", t.display);
}

/// G21：勾选翻转只改一个字符，尾注原样保留。
#[test]
fn set_checked_single_char_edit() {
    let text = fixture("days/2026-08-12.md");
    let mut tf = TodoFile::parse(&text);
    let idx = tf.tasks.iter().find(|t| !t.checked).unwrap().line_idx;
    let before = tf.lines[idx].clone();
    tf.set_checked(idx, true).unwrap();
    let after = &tf.lines[idx];
    // 恰好一个字符不同（' ' → 'x'）
    let diffs = before.chars().zip(after.chars()).filter(|(a, b)| a != b).count();
    assert_eq!(diffs, 1);
    assert_eq!(before.len(), after.len());

    // 翻回后与原行一致
    tf.set_checked(idx, false).unwrap();
    assert_eq!(tf.lines[idx], before);
}

/// 分类标签：插入→移除 往复原行一致；替换正确。
#[test]
fn category_roundtrip() {
    let line = "- [ ] (P1) 个人网站改版：重写「关于」页文案 #overdue";
    let text = format!("## 日任务\n\n{line}\n");
    let mut tf = TodoFile::parse(&text);
    let idx = tf.tasks[0].line_idx;
    assert_eq!(tf.tasks[0].category, Category::Uncategorized);

    tf.set_category(idx, Some(Category::Work)).unwrap();
    assert_eq!(tf.tasks[0].category, Category::Work);
    assert_eq!(tf.lines[idx], "- [ ] (P1) [工作] 个人网站改版：重写「关于」页文案 #overdue");

    // 换成个人
    tf.set_category(idx, Some(Category::Personal)).unwrap();
    assert_eq!(tf.lines[idx], "- [ ] (P1) [个人] 个人网站改版：重写「关于」页文案 #overdue");

    // 移除 → 回到原行
    tf.set_category(idx, None).unwrap();
    assert_eq!(tf.lines[idx], line);
}

/// 无优先级条目的分类插入位置（checkbox 之后）。
#[test]
fn category_without_priority() {
    let text = "- [ ] 无优先级任务\n";
    let mut tf = TodoFile::parse(&text);
    let idx = tf.tasks[0].line_idx;
    tf.set_category(idx, Some(Category::Personal)).unwrap();
    assert_eq!(tf.lines[idx], "- [ ] [个人] 无优先级任务");
}

/// 标签：#doing 追加幂等；#overdue 移除；#blocked 连带原因词。
#[test]
fn flag_operations() {
    let text = "- [ ] (P1) 预约牙医复诊 #blocked 等诊所回复 #overdue\n";
    let mut tf = TodoFile::parse(&text);
    let idx = tf.tasks[0].line_idx;
    let t = &tf.tasks[0];
    assert_eq!(t.blocked_reason.as_deref(), Some("等诊所回复"));
    assert!(t.overdue);
    assert!(!t.doing);

    // 幂等追加
    tf.add_flag(idx, Flag::Overdue).unwrap();
    assert_eq!(tf.lines[idx].matches("#overdue").count(), 1, "不重复追加");

    tf.add_flag(idx, Flag::Doing).unwrap();
    assert!(tf.lines[idx].ends_with("#doing"));
    assert!(tf.tasks[0].doing);

    // 移除 overdue
    tf.remove_flag(idx, Flag::Overdue).unwrap();
    assert!(!tf.tasks[0].overdue);
    assert!(!tf.lines[idx].contains("#overdue"));

    // 移除 blocked（不存在该 API 则跳过——blocked 属解析兼容范围，写入 UI 为 P1）
    // 这里验证原因词随标签一起被移除的语义在 parse 侧成立即可：
    let t = &tf.tasks[0];
    assert_eq!(t.blocked_reason.as_deref(), Some("等诊所回复"));
}

/// 注记保留：#overdue 后的非标签文本进 display。
#[test]
fn annotations_kept() {
    let text = "- [ ] (P2) 补录昨天的事项 #overdue 补记9/2\n";
    let tf = TodoFile::parse(&text);
    let t = &tf.tasks[0];
    assert!(t.overdue);
    assert_eq!(t.content, "补录昨天的事项");
    assert!(t.display.contains("补记9/2"), "注记保留在 display: {}", t.display);
}

/// 删除任务连带缩进子行。
#[test]
fn delete_with_sublines() {
    let text = "\
## 日任务

- [ ] (P1) 周报草稿：汇总本周三个项目的进展
  要求：每个项目写结论和下一步；控制在半页
  最后附里程碑表格
- [ ] (P2) 第二个任务
";
    let mut tf = TodoFile::parse(&text);
    assert_eq!(tf.tasks.len(), 2);
    assert_eq!(tf.tasks[0].sub_lines.len(), 2, "缩进续行归属上一条");

    let idx = tf.tasks[0].line_idx;
    tf.delete_task(idx).unwrap();
    assert_eq!(tf.tasks.len(), 1);
    assert!(tf.tasks[0].content.contains("第二个任务"));
    assert!(!tf.serialize().contains("要求：每个项目"), "子行一并删除");
    assert!(!tf.serialize().contains("周报草稿"), "主行删除");
}

/// 新增任务落位在二级段平铺区（不进入 ### 子区，不落入下一二级段）。
#[test]
fn add_task_flat_area() {
    let text = "\
## 日任务

- [ ] (P1) 已有的平铺任务

### ⭐ 睡前必须完成
- [x] (P1) 子区任务

## 完成事项
";
    let mut tf = TodoFile::parse(&text);
    let at = tf.add_task("日任务", Category::Work, Priority::P2, "新增的任务").unwrap();
    // 新行应在已有平铺任务之后、### 之前
    assert_eq!(tf.lines[at], "- [ ] (P2) [工作] 新增的任务");
    let pos_sub = tf.lines.iter().position(|l| l.starts_with("### ")).unwrap();
    assert!(at < pos_sub, "新增行必须在子区标题之前");
    // 视图刷新后可见
    assert!(tf.tasks.iter().any(|t| t.content == "新增的任务" && t.subsection.is_none()));
}

/// 空日文件：零任务、往返一致、可直接新增。
#[test]
fn empty_day_file() {
    let text = fixture("days/2026-08-13.md");
    let mut tf = TodoFile::parse(&text);
    assert!(tf.tasks.is_empty(), "空文件零任务");

    tf.add_task("日任务", Category::Personal, Priority::P3, "空文件里的第一条").unwrap();
    assert_eq!(tf.tasks.len(), 1);
    let t = &tf.tasks[0];
    assert_eq!(t.section, "日任务");
    assert_eq!(t.category, Category::Personal);
    assert_eq!(t.priority, Some(Priority::P3));

    // 重新解析序列化结果，结构稳定
    let tf2 = TodoFile::parse(&tf.serialize());
    assert_eq!(tf2.tasks.len(), 1);
    assert_eq!(tf2.tasks[0].content, "空文件里的第一条");
}

/// set_content 只替换内容主体，保留前缀/标签/注记。
#[test]
fn set_content_preserves_affixes() {
    let text = "- [ ] (P1) [工作] 旧内容 #blocked 等回复 #overdue 补记9/2\n";
    let mut tf = TodoFile::parse(&text);
    let idx = tf.tasks[0].line_idx;
    tf.set_content(idx, "新内容").unwrap();
    assert_eq!(tf.lines[idx], "- [ ] (P1) [工作] 新内容 #blocked 等回复 #overdue 补记9/2");
    assert_eq!(tf.tasks[0].content, "新内容");
    assert_eq!(tf.tasks[0].blocked_reason.as_deref(), Some("等回复"));
}

/// set_priority：插入（checkbox 后、分类前）/ 原地替换 / 移除字节还原（US11 P0 置顶的文件层）。
#[test]
fn set_priority_insert_swap_remove() {
    let text = "# D\n\n## 日任务\n- [ ] [个人] 无优先级任务\n- [ ] (P2) [工作] 有优先级\n";
    let mut tf = TodoFile::parse(text);
    let li0 = tf.tasks[0].line_idx;
    let li1 = tf.tasks[1].line_idx;

    // 插入：位于 checkbox 后、分类标签前
    tf.set_priority(li0, Some(Priority::P0)).unwrap();
    assert_eq!(tf.lines[li0], "- [ ] (P0) [个人] 无优先级任务");
    assert_eq!(tf.tasks[0].priority, Some(Priority::P0));

    // 等长原地替换
    tf.set_priority(li0, Some(Priority::P2)).unwrap();
    assert_eq!(tf.lines[li0], "- [ ] (P2) [个人] 无优先级任务");

    // 移除 → 整个文件字节还原
    tf.set_priority(li0, None).unwrap();
    assert_eq!(tf.serialize(), text);

    // 已有优先级的行：替换
    tf.set_priority(li1, Some(Priority::P0)).unwrap();
    assert_eq!(tf.lines[li1], "- [ ] (P0) [工作] 有优先级");

    // 已是同值：幂等，不报错
    tf.set_priority(li1, Some(Priority::P0)).unwrap();
    assert_eq!(tf.lines[li1], "- [ ] (P0) [工作] 有优先级");
}

/// F7：#blocked 增/改/删。增 = 行尾追加；改 = 原因词替换；删 = 移除标签连同原因。
#[test]
fn set_blocked_add_update_remove() {
    let text = "# D\n\n## 日任务\n- [ ] (P1) [工作] 联系供应商 #doing\n- [ ] (P2) [个人] 买咖啡 #blocked 缺现金\n";
    let mut tf = TodoFile::parse(text);
    let li0 = tf.tasks[0].line_idx;
    let li1 = tf.tasks[1].line_idx;

    // 增：行尾追加（不影响既有 #doing）
    tf.set_blocked(li0, Some("等回复")).unwrap();
    assert_eq!(tf.lines[li0], "- [ ] (P1) [工作] 联系供应商 #doing #blocked 等回复");
    assert_eq!(tf.tasks[0].blocked_reason.as_deref(), Some("等回复"));
    assert!(tf.tasks[0].doing);

    // 改：替换原因词
    tf.set_blocked(li0, Some("等二次回复")).unwrap();
    assert_eq!(tf.lines[li0], "- [ ] (P1) [工作] 联系供应商 #doing #blocked 等二次回复");
    assert_eq!(tf.tasks[0].blocked_reason.as_deref(), Some("等二次回复"));

    // 删：标签连同原因移除，前置 #doing 完好
    tf.set_blocked(li0, None).unwrap();
    assert_eq!(tf.lines[li0], "- [ ] (P1) [工作] 联系供应商 #doing");
    assert_eq!(tf.tasks[0].blocked_reason, None);

    // 既有 blocked+原因：删除后字节还原到无标签形态
    tf.set_blocked(li1, None).unwrap();
    assert_eq!(tf.lines[li1], "- [ ] (P2) [个人] 买咖啡");
    assert_eq!(tf.tasks[1].blocked_reason, None);

    // 空白原因拒绝（防止写出无原因的悬空标签）
    assert!(tf.set_blocked(li1, Some("   ")).is_err());
}

/// 删除（含缩进子行）→ 原样插回 = 字节级恢复（3s 撤销气泡的保真基础）。
#[test]
fn delete_then_restore_byte_exact() {
    let text = "# D\n\n## 日任务\n- [ ] (P1) [工作] 主任务\n  子说明行\n  另一条\n- [ ] (P2) [个人] 买咖啡\n";
    let mut tf = TodoFile::parse(text);
    let li0 = tf.tasks[0].line_idx;

    let removed = tf.delete_task(li0).unwrap();
    assert_eq!(removed.len(), 3, "应连删缩进子行");
    assert_eq!(tf.tasks.len(), 1);
    assert_eq!(tf.tasks[0].line_idx, li0, "后续任务上移到同一行号");

    // 原样插回 → 整个文件字节还原
    tf.insert_lines_at(li0, removed);
    assert_eq!(tf.serialize(), text);
    assert_eq!(tf.tasks.len(), 2);
    assert_eq!(tf.tasks[0].line_idx, li0);

    // 越界钳制到末尾，不 panic
    let r2 = tf.delete_task(tf.tasks[1].line_idx).unwrap();
    tf.insert_lines_at(usize::MAX, r2);
    assert_eq!(tf.serialize(), text);
}

/// 安全审查 H1：内容区为空、标签紧跟前缀（"- [ ] #doing"、"(P1) #blocked 原因"）
/// 曾触发反向切片 panic——release 是 panic=abort，远端推来这种行会让启动/tick 死循环。
#[test]
fn tag_first_line_no_panic_roundtrip() {
    let text = "# 2026-09-10 (周四)\n\n## 日任务\n- [ ] #doing\n- [ ] (P1) #blocked 等回复\n- [ ] (P0) [工作] #overdue\n";
    let tf = TodoFile::parse(text);
    assert_eq!(tf.tasks.len(), 3);
    assert!(tf.tasks[0].doing && tf.tasks[0].content.is_empty());
    assert_eq!(tf.tasks[1].blocked_reason.as_deref(), Some("等回复"));
    assert!(tf.tasks[1].content.is_empty());
    assert!(tf.tasks[2].overdue && tf.tasks[2].content.is_empty());
    assert_eq!(tf.serialize(), text);
}

/// 用户文本守卫：拦换行与活标签词；识别规则与 parse_line 对齐（空格后/行首 + 闭集词）。
#[test]
fn validate_user_text_rules() {
    use app_lib::parser::{validate_restore_lines, validate_user_text};
    assert!(validate_user_text("正常内容 #话题").is_ok());
    assert!(validate_user_text("提交#overdue 报告").is_ok()); // # 前不是空格，parse 同样不识别
    assert!(validate_user_text("提交 #overdue 报告").is_err());
    assert!(validate_user_text("#doing").is_err());
    assert!(validate_user_text("受阻 #blocked 原因").is_err());
    assert!(validate_user_text("带\n换行").is_err());
    assert!(validate_user_text("带\r换行").is_err());
    assert!(validate_restore_lines(&["- [ ] (P1) #blocked 原因".to_string()]).is_ok());
    assert!(validate_restore_lines(&["a\nb".to_string()]).is_err());
}
