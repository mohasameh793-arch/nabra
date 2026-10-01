// Transforms: hold Right Alt and say what to do with the selected text (or what you just dictated).
import { $, h, call, state, copyText } from "./core.js";

const EXAMPLES = [
  ["Make it shorter", "اختصرها"], ["Fix the grammar", "صحح الأخطاء"], ["Make it more professional", "خلها رسمية"],
  ["Translate to English", "ترجمها للإنجليزي"], ["Turn it into bullet points", "حطها نقاط"], ["Make it friendlier", "خلها ألطف"],
];
const COMMANDS = [
  ["“Scratch that” · «امسحها»", "Deletes what Nabra just typed"],
  ["“New line” · «سطر جديد»", "Starts a new line"],
  ["“New paragraph” · «فقرة جديدة»", "Starts a new paragraph"],
  ["“Undo” · «تراجع»", "Undo in the current app"],
  ["“Select all” · «حدد الكل»", "Selects everything in the field"],
];
let busy = false;
let result = "";

export function render() {
  const key = state.boot.keys.command;
  const text = h("textarea", { rows: 5, dir: "auto", id: "tf-text", placeholder: "Paste or dictate some text here…" }, $("#tf-text")?.value ?? "");
  const instruction = h("input", { type: "text", dir: "auto", id: "tf-instruction", placeholder: "e.g. make it shorter, ترجمها للإنجليزي",
    value: $("#tf-instruction")?.value ?? "" });
  $("#page-transforms").replaceChildren(h("div", { class: "narrow" },
    h("div", { class: "page-head" }, h("h1", {}, "Transforms")),
    h("div", { class: "card how" },
      h("ol", { class: "steps" },
        h("li", {}, h("b", {}, "Select text"), " in any app, or skip this to change what you just dictated."),
        h("li", {}, "Hold ", h("kbd", {}, key), " and say what you want."),
        h("li", {}, "Release. The text is replaced in place."))),
    h("h2", { class: "label", style: "margin:26px 0 10px" }, "Things to say"),
    h("div", { class: "chips light" }, EXAMPLES.map(([en, ar]) =>
      h("button", { class: "chip", onclick: () => { $("#tf-instruction").value = en; } }, en, h("small", { dir: "rtl" }, ` · ${ar}`)))),
    h("h2", { class: "label", style: "margin:26px 0 10px" }, "Voice commands"),
    h("div", { class: "list" }, COMMANDS.map(([say, does]) =>
      h("div", { class: "row-item word-row" }, h("div", { dir: "auto" }, say), h("span", { class: "muted" }, does)))),
    h("h2", { class: "label", style: "margin:26px 0 10px" }, "Try it here"),
    h("div", { class: "card try" }, text, h("div", { class: "row" }, instruction,
      h("button", { class: "btn primary", disabled: busy, onclick: run }, busy ? "Working…" : "Apply")),
      result ? h("div", { class: "result" }, h("div", { dir: "auto" }, result),
        h("button", { class: "btn ghost", onclick: () => copyText(result) }, "Copy")) : null),
  ));
}

async function run() {
  busy = true;
  render();
  try {
    result = await call("transform_text", { text: $("#tf-text").value, instruction: $("#tf-instruction").value });
  } finally {
    busy = false;
    render();
  }
}
