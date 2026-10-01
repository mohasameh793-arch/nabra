// Dictionary: personal words (spelled your way, with optional Arabic "sounds like" spellings) and
// replacements (btw → by the way). Saved to dictionary.json, which the engine re-reads on change.
import { $, h, iconBtn, call, state, toast } from "./core.js";

let tab = "all";
let sort = "recent";
let query = "";
let bannerClosed = localStorage.getItem("dict-banner-closed") === "1";

const isReplacement = (w) => Boolean(w.from);
const label = (w) => (isReplacement(w) ? `${w.from} → ${w.to}` : w.term);

export async function load() {
  state.words = await call("words");
}

export function render() {
  const q = query.trim().toLowerCase();
  const shown = state.words
    .filter((w) => tab === "all" || (tab === "words") !== isReplacement(w))
    .filter((w) => !q || label(w).toLowerCase().includes(q) || (w.sounds_like ?? []).some((s) => s.includes(q)));
  if (sort === "az") shown.sort((a, b) => label(a).localeCompare(label(b)));
  const recent = state.words.filter((w) => !isReplacement(w)).slice(0, 4);

  $("#page-dictionary").replaceChildren(h("div", { class: "narrow" },
    h("div", { class: "page-head" }, h("h1", {}, "Dictionary"),
      h("button", { class: "btn primary", onclick: () => edit() }, "Add new")),
    h("div", { class: "tabs", role: "tablist" },
      ...[["all", "All"], ["words", "Words"], ["replacements", "Replacements"]].map(([id, name]) =>
        h("button", { role: "tab", "aria-selected": String(tab === id), onclick: () => { tab = id; render(); } }, name)),
      h("span", { class: "grow" }),
      h("button", { class: "btn ghost", title: "Sort", onclick: () => { sort = sort === "az" ? "recent" : "az"; render(); } }, sort === "az" ? "A–Z" : "Newest"),
      h("input", { type: "search", placeholder: "Search", value: query, style: "width:180px;padding:6px 10px", "aria-label": "Search dictionary",
        oninput: (e) => { query = e.target.value; render(); $("#page-dictionary input[type=search]")?.focus(); } })),
    bannerClosed ? null : h("div", { class: "dict-banner" },
      iconBtn("close", "Dismiss", () => { bannerClosed = true; localStorage.setItem("dict-banner-closed", "1"); render(); }, { class: "icon-btn close" }),
      h("h2", {}, "Nabra spells the way ", h("em", {}, "you"), " do."),
      h("p", {}, "Add your ", h("b", {}, "names, products, and jargon"),
        ". Add how they sound in Arabic letters too, and Nabra writes them correctly in mixed Arabic and English dictation and in call notes."),
      h("div", { class: "chips" },
        h("button", { class: "chip solid", onclick: () => edit() }, "Add new word"),
        recent.map((w) => h("button", { class: "chip", onclick: () => edit(w) }, w.term)))),
    shown.length
      ? h("div", { class: "list" }, shown.map((w) => h("div", { class: "row-item word-row" },
        h("div", {}, h("div", { dir: "auto" }, label(w)),
          w.sounds_like?.length ? h("div", { class: "sub", dir: "auto" }, `sounds like: ${w.sounds_like.join("، ")}`) : null),
        h("div", { class: "actions" },
          iconBtn("edit", "Edit", () => edit(w)),
          iconBtn("trash", "Delete", async () => {
            state.words = await call("delete_word", { id: w.id });
            render();
          })))))
      : h("p", { class: "empty" }, state.words.length ? "Nothing matches." : "Your dictionary is empty. Add a word Nabra keeps getting wrong."),
  ));
}

export function openWordEditor(word = null) {
  edit(word);
}

function edit(word = null) {
  const modal = $("#word-modal");
  let kind = word && isReplacement(word) ? "replace" : "word";
  const draw = () => {
    const seg = h("div", { class: "seg" },
      h("button", { type: "button", "aria-pressed": String(kind === "word"), onclick: () => { kind = "word"; draw(); } }, "Word"),
      h("button", { type: "button", "aria-pressed": String(kind === "replace"), onclick: () => { kind = "replace"; draw(); } }, "Replacement"));
    const fields = kind === "word"
      ? [h("label", {}, "Word or name", h("input", { type: "text", name: "term", value: word?.term ?? "", required: true, dir: "auto", placeholder: "e.g. Supabase, Layla, نبرة" })),
        h("label", {}, h("span", {}, "Sounds like ", h("small", {}, "(optional; how speech-to-text writes it, comma-separated)")),
          h("input", { type: "text", name: "sounds", value: (word?.sounds_like ?? []).join(", "), dir: "auto", placeholder: "e.g. سوبابيس" }))]
      : [h("label", {}, "When I say", h("input", { type: "text", name: "from", value: word?.from ?? "", required: true, placeholder: "btw" })),
        h("label", {}, "Write", h("input", { type: "text", name: "to", value: word?.to ?? "", required: true, dir: "auto", placeholder: "by the way" }))];
    modal.replaceChildren(h("form", { class: "form", method: "dialog", onsubmit: save },
      h("h2", {}, word ? "Edit" : "Add to dictionary"), seg, ...fields,
      h("div", { class: "row" },
        h("button", { class: "btn", type: "button", onclick: () => modal.close() }, "Cancel"),
        h("button", { class: "btn primary", type: "submit" }, "Save"))));
    modal.querySelector("input")?.focus();
  };
  async function save(e) {
    e.preventDefault();
    const f = e.target;
    const payload = kind === "word"
      ? { id: word?.id ?? 0, term: f.term.value.trim(), sounds_like: f.sounds.value.split(/[,،]/).map((s) => s.trim()).filter(Boolean) }
      : { id: word?.id ?? 0, from: f.from.value.trim(), to: f.to.value.trim(), sounds_like: [] };
    if (word && isReplacement(word) !== (kind === "replace")) {
      // Changing type: replace the old entry with a new one.
      state.words = await call("delete_word", { id: word.id });
      payload.id = 0;
    }
    state.words = await call("save_word", { word: payload });
    modal.close();
    toast("Saved. Nabra uses it from your next dictation.");
    render();
  }
  draw();
  modal.showModal();
}
