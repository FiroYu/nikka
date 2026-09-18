/** 纯渲染：状态 → DOM。无副作用、无 invoke；交互由 main.ts 事件委托处理。 */

import type { Category, LeftoversDto, Notebook, NotesDto, SyncState, TaskView, ViewKind } from "./api";

const WEEKDAY_CN = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];
export const TOUCH_DEVICE = window.matchMedia('(hover: none)').matches;

export function renderSyncSettings(panel: HTMLElement): void {
  const settings = document.createElement("div");
  settings.className = "sync-settings";
  settings.innerHTML = `
    <button id="sync-settings-toggle" class="notebook-option" type="button" aria-expanded="false" aria-controls="sync-settings-form">同步设置</button>
    <form id="sync-settings-form" hidden>
      <label for="sync-repo-url">同步仓库</label>
      <input id="sync-repo-url" type="text" readonly aria-readonly="true">
      <label for="sync-pat">PAT <span id="sync-pat-set" hidden>已设置</span></label>
      <input id="sync-pat" type="password" placeholder="GitHub Fine-grained PAT" autocomplete="new-password" spellcheck="false" autocapitalize="off">
      <div class="sync-settings-actions"><button type="submit">保存</button><button id="sync-test" type="button">测试连接</button></div>
      <p id="sync-settings-status" role="status" aria-live="polite"></p>
    </form>`;
  panel.append(settings);
}

const CATEGORY_LABEL: Record<Category, string> = {
  Work: "工 作",
  Personal: "个 人",
  Uncategorized: "未分类",
};

const CATEGORY_ORDER: Category[] = ["Work", "Personal", "Uncategorized"];

/** 优先级样式档（原型 B：mono 文本，P0 红）。 */
const PRIO_CLASS: Record<string, string> = { P0: "p0", P1: "p1", P2: "p2", P3: "p3" };

export interface FoldState {
  folded: Set<Category>;
  overdueOpen: boolean;
}

export interface UiState {
  notebookId: string;
  notebooks: Notebook[];
  loadError?: string | null;
  kind: ViewKind;
  date: string;
  /** 翻日浏览的固定日期（可为未来日）；null = 跟随今天 */
  dayDate: string | null;
  leftovers: LeftoversDto | null;
  fold: FoldState;
  /** 添加框当前分类（Tab 循环：work → personal → 未分类） */
  addCat: string;
}

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  cls?: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (text !== undefined) node.textContent = text;
  return node;
}

/** "2026-09-10" → "周四"（报头小字）。 */
export function weekdayCn(date: string): string {
  return WEEKDAY_CN[new Date(date + "T12:00:00").getDay()];
}

/** "2026-09-10" → "09.10"（报头大日期，原型 B 格式）。 */
export function mmdd(date: string): string {
  const d = new Date(date + "T12:00:00");
  const mm = String(d.getMonth() + 1).padStart(2, "0");
  const dd = String(d.getDate()).padStart(2, "0");
  return `${mm}.${dd}`;
}

/** 报头右侧 mono 行："2026-W37 · D4"（ISO 周 + 周内序号，周一=1）。 */
export function isoLine(date: string): string {
  const d = new Date(date + "T12:00:00");
  const dow = d.getDay() === 0 ? 7 : d.getDay();
  const thursday = new Date(d);
  thursday.setDate(d.getDate() + (4 - dow));
  const jan1 = new Date(thursday.getFullYear(), 0, 1);
  const week = Math.floor((thursday.getTime() - jan1.getTime()) / 604800000) + 1;
  return `${thursday.getFullYear()}-W${String(week).padStart(2, "0")} · D${dow}`;
}

export function renderNotes(body: HTMLElement, dto: NotesDto): void {
  body.replaceChildren();
  for (const day of dto.kind === "day" ? dto.days.slice(0, 1) : dto.days) {
    const block = el("label", "notes-block");
    block.append(el("span", "notes-date", `${mmdd(day.date)} ${day.weekday}`));
    const input = el("textarea", "notes-input");
    input.value = day.content;
    input.dataset.noteDate = day.date;
    input.dataset.baseVersion = day.base_version;
    input.dataset.loaded = day.content;
    block.append(input);
    body.append(block);
  }
}

function renderTask(task: TaskView, showSource: boolean): HTMLElement {
  const item = el("div", "item" + (task.checked ? " done" : ""));
  item.dataset.line = String(task.row_idx);

  const prio = task.priority ? PRIO_CLASS[task.priority] : "pnone";
  const bar = el("button", `bar ${prio}`, task.priority ?? "·");
  bar.dataset.action = "prio";
  bar.title = task.priority ?? "无优先级（点击设 P0）";
  bar.setAttribute("aria-label", `切换优先级：${task.priority ?? "未设置"}`);
  item.append(bar);

  const cbx = el("button", "cbx");
  const mark = el("span", "cbx-mark", task.checked ? "✓" : "");
  mark.setAttribute("aria-hidden", "true");
  cbx.append(mark);
  cbx.dataset.action = "check";
  cbx.setAttribute("role", "checkbox");
  cbx.setAttribute("aria-checked", String(task.checked));
  cbx.setAttribute("aria-label", task.display);
  item.append(cbx);

  const body = el("div", "body");
  const txt = el("div", "txt");
  txt.textContent = task.display; // 全文自适应，不折叠
  txt.dataset.action = "edit";
  if (!task.checked) {
    txt.tabIndex = 0;
    txt.setAttribute("role", "button");
    txt.setAttribute("aria-label", `编辑：${task.display}`);
    txt.addEventListener("keydown", (event) => {
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        txt.click();
      }
    });
  }
  body.append(txt);

  if (task.sub_lines.length > 0) {
    for (const s of task.sub_lines) {
      body.append(el("div", "sub-line", s));
    }
  }

  const meta = el("div", "meta");
  if (task.source.kind === "day" && showSource) {
    meta.append(el("span", "source-date", `${mmdd(task.source.date)} ${weekdayCn(task.source.date)}`));
  }
  if (task.doing) meta.append(el("span", "tag doing-tag", "进行中"));
  if (task.overdue) meta.append(el("span", "tag overdue-tag", "逾期"));
  if (task.blocked_reason) {
    meta.append(el("span", "tag blocked-tag", `受阻: ${task.blocked_reason}`));
  }
  if (meta.childElementCount > 0) body.append(meta);
  item.append(body);

  const del = el("button", "del");
  del.textContent = "✕";
  del.dataset.action = "del";
  del.title = "删除";
  del.setAttribute("aria-label", `删除：${task.display}`);
  item.append(del);
  return item;
}

function renderGroup(cat: Category, tasks: TaskView[], folded: boolean, showSource: boolean): HTMLElement {
  const group = el("section", "group");
  group.dataset.cat = cat;

  const head = el("button", "fold-btn");
  head.dataset.action = "fold";
  head.setAttribute("aria-expanded", String(!folded));
  head.append(el("span", "fold-title", CATEGORY_LABEL[cat]));
  const undone = tasks.filter((t) => !t.checked).length;
  head.append(el("span", "fold-count", `${undone} 项未完成`));
  head.append(el("span", "fold-arrow" + (folded ? "" : " open"), "▸"));
  group.append(head);

  if (!folded) {
    const list = el("div", "list");
    // 未完成在前，完成后划线沉底（同组内稳定排序）
    const ordered = [...tasks.filter((t) => !t.checked), ...tasks.filter((t) => t.checked)];
    for (const t of ordered) list.append(renderTask(t, showSource));
    group.append(list);
  }
  return group;
}

function renderStar(tasks: TaskView[], showSource: boolean): HTMLElement {
  const sec = el("section", "star-sec");
  sec.append(el("div", "star-title", "⭐ 睡前必须完成"));
  const list = el("div", "list");
  for (const t of tasks) list.append(renderTask(t, showSource));
  sec.append(list);
  return sec;
}

/** F23 本周统计：完成 N/M + 进度条（纯前端由视图计算，week 视图）。 */
function renderStats(root: HTMLElement, kind: ViewKind, tasks: TaskView[]): void {
  const box = root.querySelector<HTMLElement>("#stats")!;
  if (kind !== "week" || tasks.length === 0) {
    box.hidden = true;
    return;
  }
  const done = tasks.filter((t) => t.checked).length;
  const pct = Math.round((done / tasks.length) * 100);
  box.hidden = false;
  box.replaceChildren();
  box.append(el("span", undefined, `本周 ${done}/${tasks.length} 完成`));
  const bar = el("div", "stats-bar");
  const fill = el("div", "stats-fill");
  fill.style.width = `${pct}%`;
  bar.append(fill);
  box.append(bar, el("span", "stats-pct", `${pct}%`));
}

/** F14 提示条 + 逾期聚合（day 视图）。 */
function renderCarry(
  root: HTMLElement,
  leftovers: LeftoversDto | null,
  open: boolean,
): void {
  const carry = root.querySelector<HTMLElement>("#carry")!;
  if (!leftovers || leftovers.count === 0) {
    carry.hidden = true;
    return;
  }
  carry.hidden = false;
  carry.replaceChildren();

  const line = el("div", "carry-line");
  line.textContent = `昨天还有 ${leftovers.count} 项未完成`;
  const btn = el("button", "carry-btn");
  btn.textContent = open ? "收起" : "复制到今天";
  btn.dataset.action = "carry-toggle";
  line.append(btn);
  carry.append(line);

  if (open) {
    for (const t of leftovers.tasks) {
      const row = el("div", "carry-row");
      row.textContent = t.content;
      const copy = el("button", "carry-copy");
      copy.textContent = "＋";
      copy.title = "复制到今天";
      copy.dataset.action = "carry-copy";
      copy.dataset.line = String(t.line_idx);
      row.append(copy);
      carry.append(row);
    }
  }
}

function renderOverdueSum(root: HTMLElement, tasks: TaskView[]): void {
  const sum = root.querySelector<HTMLElement>("#overdue-sum")!;
  const n = tasks.filter((t) => !t.checked && t.overdue).length;
  if (n === 0) {
    sum.hidden = true;
    return;
  }
  sum.hidden = false;
  sum.replaceChildren();
  const span = el("span", undefined, `${n} 项标注了 #overdue`);
  const note = el("span", "sum-note", "（只读）");
  sum.append(span, note);
}

/** 整体重渲（v1 不做 keyed-diff；列表规模 ~几十，重渲足够快且无输入态冲突）。 */
export function render(
  root: HTMLElement,
  state: UiState,
  view: { date: string; relPath: string; tasks: TaskView[] } | null,
): void {
  root.querySelector<HTMLElement>("#head-weekday")!.textContent = view
    ? weekdayCn(view.date)
    : "—";
  root.querySelector<HTMLElement>("#head-date")!.textContent = view
    ? mmdd(view.date)
    : "--.--";
  root.querySelector<HTMLElement>("#head-iso")!.textContent = view
    ? isoLine(view.date)
    : "";

  // 翻日导航：仅日视图显示；‹ › 恒显（可向前也可向未来），
  // 「回今天」在固定浏览某日（历史或未来）时才出现
  const isDay = state.kind === "day";
  root.querySelector<HTMLElement>("#nav-prev")!.hidden = !isDay;
  root.querySelector<HTMLElement>("#nav-next")!.hidden = !isDay;
  root.querySelector<HTMLElement>("#head-today")!.hidden = !isDay || state.dayDate === null;
  if (TOUCH_DEVICE) {
    const today = root.querySelector<HTMLElement>("#head-today")!;
    today.textContent = "今";
    today.setAttribute("aria-label", "回今天");
  }

  for (const k of ["day", "week"] as ViewKind[]) {
    root.querySelector<HTMLElement>(`#tab-${k}`!)?.classList.toggle(
      "active",
      state.kind === k,
    );
    root.querySelector<HTMLElement>(`#tab-${k}`)?.setAttribute(
      "aria-selected",
      String(state.kind === k),
    );
  }

  // F14 遗留条只属于跟随今天的日视图（翻历史/未来日时无「昨天遗留」语义）
  renderCarry(root, state.kind === "day" && !state.dayDate ? state.leftovers : null, state.fold.overdueOpen);
  renderStats(root, state.kind, view?.tasks ?? []);

  const groups = root.querySelector<HTMLElement>("#groups")!;
  groups.replaceChildren();
  renderOverdueSum(root, view?.tasks ?? []);

  if (TOUCH_DEVICE && state.loadError) {
    const failure = el("div", "empty load-error");
    failure.setAttribute("role", "alert");
    const retry = el("button", "load-retry", "重试");
    retry.type = "button";
    retry.dataset.action = "retry-load";
    failure.append(el("p", undefined, state.loadError), retry);
    groups.append(failure);
  } else if (TOUCH_DEVICE && view && view.tasks.length === 0) {
    const isToday = isDay && state.dayDate === null;
    const empty = el("div", "empty touch-empty",
      !isDay ? "本周暂无事项" : isToday ? "今天还没有事项 · 点右下角记一条" : "这一天没有记录");
    if (isToday) {
      const cue = el("span", "empty-fab-cue", "↘");
      cue.setAttribute("aria-hidden", "true");
      empty.append(cue);
    }
    groups.append(empty);
  } else if (!view || view.tasks.length === 0) {
    groups.append(
      el(
        "div",
        "empty",
        !view ? "正在加载事项…" : state.kind === "week" ? "本周还没有事项，记一条吧" : state.dayDate ? "这天没有记录" : "今天还没有事项，记一条吧",
      ),
    );
  } else {
    // ⭐ 子区条目单独归入 star-sec（保留原顺序），其余按分类三组
    const star = view.tasks.filter((t) => t.subsection !== null);
    const rest = view.tasks.filter((t) => t.subsection === null);
    for (const cat of CATEGORY_ORDER) {
      const inCat = rest.filter((t) => t.category === cat);
      if (inCat.length === 0) continue;
      groups.append(renderGroup(cat, inCat, state.fold.folded.has(cat), !isDay));
    }
    if (star.length > 0) groups.append(renderStar(star, !isDay));
  }

  if (view && view.tasks.length === 0 && !state.loadError) {
    const others = state.notebooks.filter(book => book.id !== state.notebookId);
    if (others.length > 0) {
      const hint = el("div", "empty-notebooks", "其他笔记本可能有你的事项：");
      others.forEach((book, index) => {
        if (index > 0) hint.append("、");
        const button = el("button", "empty-notebook-link", book.name);
        button.type = "button";
        button.dataset.action = "switch-notebook";
        button.dataset.notebookId = book.id;
        hint.append(button);
      });
      groups.querySelector(".empty")?.append(hint);
    }
  }

  renderAdd(root, state);
}

/** 添加入口（.add 按钮 / 展开输入框）。 */
export function renderAdd(root: HTMLElement, state: { addCat: string }): void {
  const slot = root.querySelector<HTMLElement>("#add-slot")!;
  slot.replaceChildren();
  const btn = el("button", "add");
  btn.append("＋ 记一条…", el("span", "desktop-hint", `（Enter 落 [${state.addCat}]，Tab 切分类）`));
  if (TOUCH_DEVICE) {
    btn.classList.add("add-fab");
    btn.setAttribute("aria-label", btn.textContent!);
  }
  btn.dataset.action = "add";
  slot.append(btn);
}

/** 同步角标。 */
export function renderSync(root: HTMLElement, s: SyncState | null): void {
  const dot = root.querySelector<HTMLElement>("#sync-dot")!;
  const text = root.querySelector<HTMLElement>("#sync-text")!;
  text.removeAttribute("title");
  dot.dataset.state = s?.kind ?? "idle";
  switch (s?.kind) {
    case "syncing":
      text.textContent = "同步中…";
      break;
    case "offline":
      text.textContent = `离线 · ${s.unpushed} 条待推`;
      break;
    case "conflict":
      text.textContent = "冲突：需人工处理";
      text.title = s.message;
      break;
    case "error":
      text.textContent = "同步异常";
      text.title = s.message;
      break;
    default:
      text.textContent = "已同步";
  }
}
