// Shared helpers for the hub pages.
export const invoke = window.__TAURI__.core.invoke;
export const listen = window.__TAURI__.event.listen;
export const $ = (sel, root = document) => root.querySelector(sel);

/** Tiny element builder: h("div", { class: "x", onclick }, child, "text"). Text is always set safely. */
export function h(tag, attrs = {}, ...children) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v == null || v === false) continue;
    if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else if (k === "html") el.innerHTML = v; // only ever used with our own static SVG strings
    else el.setAttribute(k, v === true ? "" : v);
  }
  el.append(...children.flat().filter((c) => c != null && c !== false));
  return el;
}

export const ICONS = {
  copy: '<svg viewBox="0 0 24 24"><rect x="8" y="8" width="11" height="11" rx="2"/><path d="M5 15V6.5A1.5 1.5 0 0 1 6.5 5H15"/></svg>',
  flag: '<svg viewBox="0 0 24 24"><path d="M6 21V4.5M6 4.5h10l-2 4 2 4H6"/></svg>',
  more: '<svg viewBox="0 0 24 24"><circle cx="12" cy="6" r="1.2" class="fill"/><circle cx="12" cy="12" r="1.2" class="fill"/><circle cx="12" cy="18" r="1.2" class="fill"/></svg>',
  trash: '<svg viewBox="0 0 24 24"><path d="M5 7h14M10 7V5h4v2M7 7l1 12h8l1-12"/></svg>',
  edit: '<svg viewBox="0 0 24 24"><path d="M5 19h4L19 9l-4-4L5 15z"/></svg>',
  search: '<svg viewBox="0 0 24 24"><circle cx="11" cy="11" r="6"/><path d="M20 20l-4.5-4.5"/></svg>',
  close: '<svg viewBox="0 0 24 24"><path d="M7 7l10 10M17 7L7 17"/></svg>',
  doc: '<svg viewBox="0 0 24 24"><path d="M7 3.5h7l4 4v13H7z"/><path d="M9.5 12h6M9.5 15.5h6"/></svg>',
  record: '<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="8.5"/><circle cx="12" cy="12" r="4" class="fill"/></svg>',
  gear: '<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="3"/><path d="M12 3v2.5M12 18.5V21M3 12h2.5M18.5 12H21M5.6 5.6l1.8 1.8M16.6 16.6l1.8 1.8M5.6 18.4l1.8-1.8M16.6 7.4l1.8-1.8"/></svg>',
  send: '<svg viewBox="0 0 24 24"><path d="M5 12h13M13 6l6 6-6 6"/></svg>',
  stop: '<svg viewBox="0 0 24 24"><rect x="7" y="7" width="10" height="10" rx="2" class="fill"/></svg>',
};
export const iconBtn = (icon, label, onclick, extra = {}) =>
  h("button", { class: "icon-btn", "aria-label": label, title: label, html: ICONS[icon], onclick, ...extra });

export function toast(message) {
  const t = $("#toast");
  t.textContent = message;
  t.hidden = false;
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => (t.hidden = true), 3200);
}

/** invoke() that shows the error to the user and re-throws. */
export async function call(cmd, args) {
  try {
    return await invoke(cmd, args);
  } catch (e) {
    toast(String(e));
    throw e;
  }
}

export async function copyText(text, what = "Copied") {
  await call("copy_text", { text });
  toast(what);
}

// ---------- time ----------
const startOfDay = (d) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
export const dayKey = (ms) => startOfDay(new Date(ms));
export function dayLabel(ms, style = "word") {
  const today = startOfDay(new Date());
  const day = startOfDay(new Date(ms));
  if (day === today) return "Today";
  if (day === today - 86_400_000) return "Yesterday";
  return new Date(ms).toLocaleDateString(undefined, style === "word"
    ? { weekday: "long", month: "short", day: "numeric" }
    : { weekday: "short", month: "short", day: "numeric" });
}
export const timeOf = (ms) => new Date(ms).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" }).toLowerCase().replace(" ", "");
export const clock = (s) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, "0")}`;
export const fmt = (n) => Math.round(n).toLocaleString();

/** Group items (newest first) by calendar day → [[label, items], ...]. */
export function byDay(items, getMs, style) {
  const groups = new Map();
  for (const it of items) {
    const k = dayKey(getMs(it));
    if (!groups.has(k)) groups.set(k, [dayLabel(getMs(it), style), []]);
    groups.get(k)[1].push(it);
  }
  return [...groups.values()];
}

// ---------- languages ----------
export const LANGUAGES = [
  ["ar", "Arabic", "العربية"], ["en", "English", "English"], ["fr", "French", "Français"], ["es", "Spanish", "Español"],
  ["de", "German", "Deutsch"], ["ja", "Japanese", "日本語"], ["tr", "Turkish", "Türkçe"], ["ur", "Urdu", "اردو"],
  ["hi", "Hindi", "हिन्दी"], ["fa", "Persian", "فارسی"], ["zh", "Chinese", "中文"], ["ko", "Korean", "한국어"],
  ["ru", "Russian", "Русский"], ["pt", "Portuguese", "Português"], ["it", "Italian", "Italiano"], ["id", "Indonesian", "Bahasa Indonesia"],
];
export const languageName = (code) => LANGUAGES.find(([c]) => c === code)?.[1] ?? code;
export function languageSelect(selected, autoLabel = "Auto (the call's language)") {
  const sel = h("select", {}, h("option", { value: "" }, autoLabel),
    LANGUAGES.map(([c, en, native]) => h("option", { value: c }, en === native ? en : `${en} · ${native}`)));
  sel.value = selected ?? "";
  return sel;
}

// ---------- markdown (summaries) ----------
const escapeHtml = (s) => s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

/** Small Markdown subset for AI summaries. Everything is escaped first: model output can't inject HTML. */
export function markdown(md) {
  const inline = (s) => escapeHtml(s).replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>");
  let html = "", list = false;
  const close = () => { if (list) { html += "</ul>"; list = false; } };
  for (const raw of md.split("\n")) {
    const line = raw.trim();
    let m;
    if (!line) close();
    else if ((m = line.match(/^#{1,6}\s+(.+)$/))) { close(); html += `<h4 dir="auto">${inline(m[1])}</h4>`; }
    else if ((m = line.match(/^[-*]\s+\[( |x)\]\s+(.+)$/i))) {
      if (!list) { html += "<ul>"; list = true; }
      html += `<li class="task" dir="auto"><input type="checkbox"${m[1] === " " ? "" : " checked"} aria-label="Done"><span>${inline(m[2])}</span></li>`;
    } else if ((m = line.match(/^[-*]\s+(.+)$/))) {
      if (!list) { html += "<ul>"; list = true; }
      html += `<li dir="auto">${inline(m[1])}</li>`;
    } else { close(); html += `<p dir="auto">${inline(line)}</p>`; }
  }
  close();
  return html;
}

// ---------- appearance (a per-PC preference, kept in localStorage) ----------
export function getTheme() {
  try { return localStorage.getItem("theme") || "light"; } catch { return "light"; }
}
export function setTheme(theme) {
  document.documentElement.dataset.theme = theme;
  try { localStorage.setItem("theme", theme); } catch { /* storage unavailable: still applied for this session */ }
}

// ---------- shared state ----------
export const state = { boot: null, settings: null, history: [], words: [], notifications: [] };

export function notify(text, detail = "") {
  state.notifications.unshift({ text, detail, at: Date.now() });
  document.dispatchEvent(new CustomEvent("notified"));
}
