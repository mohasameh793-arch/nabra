// Hub shell: window controls, sidebar navigation, notifications, and wiring backend events to pages.
import { $, h, call, listen, state, toast, getTheme, setTheme } from "./js/core.js";
import * as dictation from "./js/dictation.js";
import * as notetaker from "./js/notetaker.js";
import * as insights from "./js/insights.js";
import * as dictionary from "./js/dictionary.js";
import * as snippets from "./js/snippets.js";
import * as style from "./js/style.js";
import * as transforms from "./js/transforms.js";
import * as scratchpad from "./js/scratchpad.js";
import * as setup from "./js/setup.js";
import { openSettings, openHelp } from "./js/settings.js";

setTheme(getTheme());

const PAGES = { dictation, notetaker, insights, dictionary, snippets, style, transforms, scratchpad, setup };
let current = "dictation";

export function go(page) {
  if (!PAGES[page]) page = "dictation";
  current = page;
  for (const p of Object.keys(PAGES)) $(`#page-${p}`).hidden = p !== page;
  document.querySelectorAll(".sidebar a.nav").forEach((a) => a.setAttribute("aria-current", a.dataset.page === page ? "page" : "false"));
  PAGES[page].render();
  $("#sheet").scrollTop = 0;
}

// ---------- title bar ----------
const win = window.__TAURI__.window.getCurrentWindow();
$("#win-min").addEventListener("click", () => win.minimize());
$("#win-max").addEventListener("click", () => win.toggleMaximize());
$("#win-close").addEventListener("click", () => call("hide_hub")); // keeps running in the tray
$("#toggle-sidebar").addEventListener("click", () => document.body.classList.toggle("compact"));
$("#profile").addEventListener("click", () => openSettings("general"));
$("#open-settings").addEventListener("click", () => openSettings("general"));
$("#open-help").addEventListener("click", openHelp);

// ---------- notifications ----------
function drawNotifications() {
  $("#bell-badge").hidden = !state.notifications.some((n) => !n.seen);
  $("#notification-empty").hidden = state.notifications.length > 0;
  $("#notification-list").replaceChildren(...state.notifications.slice(0, 20).map((n) =>
    h("li", {}, h("span", { dir: "auto" }, n.text), n.detail ? h("small", { dir: "auto" }, n.detail) : null)));
}
$("#bell").addEventListener("click", (e) => {
  e.stopPropagation();
  const pop = $("#notifications");
  pop.hidden = !pop.hidden;
  state.notifications.forEach((n) => (n.seen = true));
  drawNotifications();
});
document.addEventListener("click", (e) => { if (!e.target.closest("#notifications")) $("#notifications").hidden = true; });
document.addEventListener("notified", drawNotifications);

// ---------- navigation ----------
document.querySelectorAll(".sidebar a.nav").forEach((a) => a.addEventListener("click", (e) => { e.preventDefault(); go(a.dataset.page); }));
document.addEventListener("settings-changed", () => PAGES[current].render());
document.addEventListener("history-cleared", () => PAGES[current].render());

// ---------- backend events ----------
listen("goto", (e) => go(e.payload));
listen("dictation", (e) => { dictation.added(e.payload); if (current === "insights") insights.render(); });
listen("engine", (e) => { state.boot.engine = e.payload; });
notetaker.wire();
setup.wire();

// ---------- boot ----------
(async () => {
  state.boot = await call("boot");
  state.settings = state.boot.settings;
  [state.history] = await Promise.all([call("history"), dictionary.load(), notetaker.load(), snippets.load(), scratchpad.load(), setup.load()]);
  if (state.boot.meeting) {
    notetaker.startLive(state.boot.meeting, await call("live_lines"));
    $("#nav-live").hidden = false;
  }
  go(state.boot.setup_ready ? location.hash.slice(1) || "dictation" : "setup");
  drawNotifications();
})().catch((e) => toast(`Couldn't load Nabra: ${e}`));
