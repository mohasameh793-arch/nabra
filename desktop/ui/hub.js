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
// "settings:<section>" opens the settings dialog (e.g. from the pill's "Connect calendar").
listen("goto", (e) => (e.payload.startsWith("settings:") ? openSettings(e.payload.slice(9)) : go(e.payload)));
listen("dictation", (e) => { dictation.added(e.payload); if (current === "insights") insights.render(); });
listen("engine", (e) => { state.boot.engine = e.payload; });
notetaker.wire();
setup.wire();

// ---------- update banner: "Nabra X is available · Update" → progress → restart ----------
const banner = h("div", { class: "update-banner", hidden: true, role: "status" });
document.body.append(banner);
function showUpdate(u) {
  banner.hidden = false;
  if (u.status === "available") {
    banner.replaceChildren(h("span", {}, "Nabra ", h("b", {}, u.version), " is available"),
      h("button", { class: "btn primary", onclick: () => call("install_update").catch(() => {}) }, "Update"),
      h("button", { class: "x", "aria-label": "Later", onclick: () => (banner.hidden = true) }, "✕"));
  } else if (u.status === "downloading") {
    banner.replaceChildren(h("span", {}, `Updating to ${u.version}… ${u.percent}%`),
      h("div", { class: "bar" }, h("i", { style: `width:${u.percent}%` })));
  } else if (u.status === "installing") {
    banner.replaceChildren(h("span", {}, "Installing… Nabra will restart"), h("div", { class: "bar" }, h("i", { style: "width:100%" })));
  } else if (u.status === "failed") {
    banner.replaceChildren(h("span", {}, u.message),
      h("button", { class: "btn", onclick: () => call("install_update").catch(() => {}) }, "Try again"),
      h("button", { class: "x", "aria-label": "Close", onclick: () => (banner.hidden = true) }, "✕"));
  }
}
listen("update", (e) => showUpdate(e.payload));

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
  if (state.boot.update) showUpdate({ status: "available", version: state.boot.update });
})().catch((e) => toast(`Couldn't load Nabra: ${e}`));
