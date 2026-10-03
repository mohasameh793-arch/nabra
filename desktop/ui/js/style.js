// Style: how dictated text is written in each kind of app. Only capitals and end punctuation change,
// never your words, so it's applied without AI (engine/shortcuts.py → apply_style).
import { $, h, call, state, toast } from "./core.js";

const KINDS = [
  ["personal", "Personal messages", "WhatsApp, Telegram, Discord, Messenger, Instagram"],
  ["work", "Work messages", "Slack, Microsoft Teams, Zoom chat"],
  ["email", "Email", "Outlook, Gmail, Proton Mail, Thunderbird"],
  ["other", "Everything else", "Docs, notes, code, AI chats, browsers"],
];
const STYLES = [
  ["formal", "Formal", "Caps + punctuation", "Hey, are you free for lunch tomorrow? Let's do 12 if that works for you."],
  ["casual", "Casual", "Caps + less punctuation", "Hey, are you free for lunch tomorrow? Let's do 12 if that works for you"],
  ["very_casual", "Very casual", "No caps + less punctuation", "hey, are you free for lunch tomorrow? let's do 12 if that works for you"],
];

export function render() {
  const styles = state.settings.styles;
  $("#page-style").replaceChildren(h("div", { class: "narrow" },
    h("div", { class: "page-head" }, h("h1", {}, "Style")),
    h("p", { class: "muted", style: "margin-top:-12px" }, "Nabra notices which app you're dictating into and writes the way you would there. ",
      "Only capitalization and end punctuation change: your words never do."),
    KINDS.map(([kind, title, apps]) => h("div", { class: "card style-card" },
      h("div", { class: "style-head" }, h("div", {}, h("h3", {}, title), h("p", { class: "muted" }, apps))),
      h("div", { class: "style-options" }, STYLES.map(([id, name, hint, sample]) =>
        h("button", { class: "style-option", "aria-pressed": String(styles[kind] === id), onclick: () => choose(kind, id) },
          h("b", {}, name), h("small", {}, hint), h("span", { class: "sample" }, sample)))))),
  ));
}

async function choose(kind, id) {
  const next = { ...state.settings, styles: { ...state.settings.styles, [kind]: id } };
  state.settings = await call("save_settings", { settings: next }); // the app returns what it actually saved
  toast("Style saved");
  render();
}
