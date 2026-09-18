/** 本机外观偏好独立于任务数据，切换不会重绘清单或打断输入。 */
export {};
const themes = [
  { id: "classic", name: "极简黑标", paper: "#f4f3f0", ink: "#16181a", detail: "素纸 · 黑墨" },
  { id: "kraft", name: "牛皮手帐", paper: "#eee0c6", ink: "#513c2c", detail: "牛皮纸 · 棕墨" },
  { id: "cream", name: "奶油横线", paper: "#fff9e9", ink: "#544633", detail: "横线纸 · 蜜金" },
  { id: "sage", name: "鼠尾草格纸", paper: "#edf2e7", ink: "#304f41", detail: "方格纸 · 森绿" },
  { id: "rose", name: "樱粉手帐", paper: "#fbefef", ink: "#704653", detail: "装订边 · 玫瑰" },
  { id: "midnight", name: "午夜墨蓝", paper: "#1d2837", ink: "#edf1f7", detail: "深色纸 · 银墨" },
  { id: "mist", name: "雾蓝点阵", paper: "#edf3f8", ink: "#355571", detail: "点阵纸 · 雾蓝" },
] as const;

const storageKey = "sticky-todo.theme";
const options = document.querySelector<HTMLElement>("#theme-options")!;
const panel = document.querySelector<HTMLDetailsElement>("#appearance")!;
const themeStatus = document.querySelector<HTMLElement>("#theme-status")!;

function applyTheme(id: string, persist: boolean): void {
  const theme = themes.find((t) => t.id === id) ?? themes[0];
  document.documentElement.dataset.theme = theme.id;
  document.querySelector<HTMLElement>("#theme-name")!.textContent = theme.name;
  options.querySelectorAll<HTMLButtonElement>("button").forEach((button) => {
    button.setAttribute("aria-pressed", String(button.dataset.theme === theme.id));
  });
  if (persist) {
    try {
      localStorage.setItem(storageKey, theme.id);
      themeStatus.textContent = "已保存 · 仅用于这台设备";
    } catch {
      themeStatus.textContent = "已切换；当前无法保存，重启后将恢复默认";
    }
  }
}

for (const theme of themes) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "theme-option";
  button.dataset.theme = theme.id;
  button.title = theme.detail;
  button.style.setProperty("--swatch-paper", theme.paper);
  button.style.setProperty("--swatch-ink", theme.ink);
  const swatch = document.createElement("span");
  swatch.className = "theme-swatch";
  swatch.setAttribute("aria-hidden", "true");
  swatch.textContent = "Aa";
  const label = document.createElement("span");
  label.textContent = theme.name;
  button.append(swatch, label);
  button.addEventListener("click", () => applyTheme(theme.id, true));
  options.append(button);
}

panel.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && panel.open) {
    event.stopPropagation();
    panel.open = false;
    panel.querySelector("summary")!.focus();
  }
});

let saved = "classic";
try { saved = localStorage.getItem(storageKey) ?? saved; } catch { /* 当前会话仍可切换外观。 */ }
applyTheme(saved, false);
