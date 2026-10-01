// Dictation (home): welcome, banner, stats, and the history of everything you've dictated.
import { $, h, ICONS, iconBtn, call, copyText, byDay, timeOf, fmt, state, toast } from "./core.js";
import { totals, streaks, appName } from "./stats.js";
import { go } from "../hub.js";
import { openWordEditor } from "./dictionary.js";

let query = "";
let searching = false;
let tipClosed = localStorage.getItem("tip-notes-closed") === "1";

export function render() {
  const page = $("#page-dictation");
  const { words, wpm } = totals(state.history);
  const { current } = streaks(state.history);
  const name = state.settings.name?.trim();

  const shown = query ? state.history.filter((d) => d.text.toLowerCase().includes(query.toLowerCase())) : state.history;

  page.replaceChildren(
    h("h2", { class: "welcome" }, name ? `Welcome back, ${name}` : "Welcome back"),
    h("div", { class: "home" },
      h("div", {},
        h("div", { class: "banner" },
          h("h2", {}, "Make Nabra spell like ", h("em", {}, "you")),
          h("p", {}, "Add the names, products and jargon you use, in Arabic or English. Nabra gets them right every time."),
          h("button", { class: "btn", onclick: () => go("dictionary") }, "Start now")),
        historyBlock(shown)),
      h("aside", { class: "card" },
        h("div", { class: "stats" },
          stat(fmt(words), "total words"), stat(fmt(wpm), "wpm"), stat(current, "day streak")),
        tipClosed ? null : h("div", { class: "side-card" },
          iconBtn("close", "Dismiss", () => { tipClosed = true; localStorage.setItem("tip-notes-closed", "1"); render(); }, { class: "icon-btn close" }),
          h("h4", {}, "Take notes in your calls"),
          h("p", {}, `Nabra writes down what they say and what you say, then summarizes it. Press ${state.boot.keys.notes}.`),
          h("button", { class: "btn cta", onclick: () => go("notetaker") }, "Open Notetaker")))),
  );
}

const stat = (value, label) => h("div", { class: "stat" }, h("b", {}, String(value)), h("span", {}, label));

function historyBlock(items) {
  const search = h("div", { class: "search" });
  const input = h("input", { type: "search", placeholder: "Search dictations", value: query, "aria-label": "Search dictations",
    oninput: (e) => { query = e.target.value; render(); $("#page-dictation input[type=search]")?.focus(); } });
  search.append(searching || query ? input : iconBtn("search", "Search", () => {
    searching = true;
    render();
    $("#page-dictation input[type=search]").focus();
  }));

  if (!state.history.length) {
    return h("div", {},
      h("div", { class: "day" }, h("span", { class: "label" }, "Today")),
      h("div", { class: "list" }, h("p", { class: "empty" },
        `Hold ${state.boot.keys.talk} anywhere and talk. Everything you dictate shows up here.`)));
  }
  const groups = byDay(items, (d) => d.id);
  return h("div", {},
    groups.length ? groups.map(([label, rows], i) => h("div", {},
      h("div", { class: "day" }, h("span", { class: "label" }, label), i === 0 ? search : null),
      h("div", { class: "list" }, rows.map(row))))
      : [h("div", { class: "day" }, h("span", { class: "label" }, "No matches"), search)],
  );
}

function row(d) {
  const flag = iconBtn("flag", d.flagged ? "Unflag" : "Flag a wrong transcription", async () => {
    await call("edit_dictation", { id: d.id, flagged: !d.flagged, delete: false });
    d.flagged = !d.flagged;
    toast(d.flagged ? "Flagged. It'll be reviewed in the accuracy benchmark." : "Unflagged");
    render();
  }, { class: `icon-btn${d.flagged ? " on" : ""}` });
  const remove = iconBtn("trash", "Delete", async () => {
    await call("edit_dictation", { id: d.id, flagged: null, delete: true });
    state.history = state.history.filter((x) => x.id !== d.id);
    render();
  });
  return h("div", { class: `row-item${d.flagged ? " flagged" : ""}` },
    h("div", {}, h("time", {}, timeOf(d.id)), d.app ? h("div", { class: "app" }, appName(d.app)) : null),
    h("div", { class: "text", dir: "auto" }, d.text),
    h("div", { class: "actions" }, iconBtn("copy", "Copy", () => copyText(d.text)),
      iconBtn("edit", "A word came out wrong? Add it to the dictionary", () => openWordEditor()), flag, remove));
}

export function added(entry) {
  state.history.unshift(entry);
  if (!$("#page-dictation").hidden) render();
}
