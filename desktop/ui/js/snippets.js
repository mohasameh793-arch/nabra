// Snippets: say a short trigger, Nabra types the saved text. Saved to snippets.json; the engine re-reads it.
import { $, h, iconBtn, call, state, toast } from "./core.js";

let items = [];
let query = "";

export async function load() {
  items = await call("snippets");
}

export function render() {
  const q = query.trim().toLowerCase();
  const shown = items.filter((s) => !q || s.trigger.toLowerCase().includes(q) || s.text.toLowerCase().includes(q));
  $("#page-snippets").replaceChildren(h("div", { class: "narrow" },
    h("div", { class: "page-head" }, h("h1", {}, "Snippets"), h("button", { class: "btn primary", onclick: () => edit() }, "Add new")),
    h("div", { class: "dict-banner snip-banner" },
      h("h2", {}, "Say less, ", h("em", {}, "type more"), "."),
      h("p", {}, "Save text you type often: your email, address, a meeting link, a signature, a whole reply. ",
        `Hold ${state.boot.keys.talk} and say its trigger. Nabra types the full text exactly as saved.`),
      h("div", { class: "chips" },
        h("button", { class: "chip solid", onclick: () => edit() }, "Add a snippet"),
        ["my email", "my address", "توقيعي", "meeting link"].map((t) => h("button", { class: "chip", onclick: () => edit({ id: 0, trigger: t, text: "" }) }, t)))),
    h("div", { class: "tabs" }, h("button", { "aria-selected": "true" }, `All snippets · ${items.length}`), h("span", { class: "grow" }),
      h("input", { type: "search", placeholder: "Search", value: query, style: "width:180px;padding:6px 10px", "aria-label": "Search snippets",
        oninput: (e) => { query = e.target.value; render(); $("#page-snippets input[type=search]")?.focus(); } })),
    shown.length
      ? h("div", { class: "list" }, shown.map((s) => h("div", { class: "row-item word-row" },
        h("div", {}, h("div", { dir: "auto" }, h("b", {}, `“${s.trigger}”`)), h("div", { class: "sub snippet-body", dir: "auto" }, s.text)),
        h("div", { class: "actions" },
          iconBtn("edit", "Edit", () => edit(s)),
          iconBtn("trash", "Delete", async () => { items = await call("delete_snippet", { id: s.id }); render(); })))))
      : h("p", { class: "empty" }, items.length ? "Nothing matches." : "No snippets yet."),
  ));
}

function edit(snippet = null) {
  const modal = $("#word-modal");
  modal.replaceChildren(h("form", { class: "form", onsubmit: async (e) => {
    e.preventDefault();
    const f = e.target;
    items = await call("save_snippet", { snippet: { id: snippet?.id ?? 0, trigger: f.trigger.value.trim(), text: f.text.value } });
    modal.close();
    toast("Saved. Say the trigger to use it.");
    render();
  } },
    h("h2", {}, snippet?.id ? "Edit snippet" : "New snippet"),
    h("label", {}, h("span", {}, "Trigger ", h("small", {}, "(what you'll say, in any language)")),
      h("input", { type: "text", name: "trigger", value: snippet?.trigger ?? "", required: true, maxlength: 100, dir: "auto", placeholder: "my email" })),
    h("label", {}, "Text to insert",
      h("textarea", { name: "text", rows: 7, required: true, dir: "auto", placeholder: "majesty@example.com" }, snippet?.text ?? "")),
    h("div", { class: "row" },
      h("button", { class: "btn", type: "button", onclick: () => modal.close() }, "Cancel"),
      h("button", { class: "btn primary", type: "submit" }, "Save"))));
  modal.showModal();
  modal.querySelector(snippet?.trigger ? "textarea" : "input").focus();
}
