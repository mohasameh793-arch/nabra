// Scratchpad: quick drafts inside Nabra. Click into a pad and dictate. Saved automatically.
import { $, h, iconBtn, call, state, copyText, timeOf, dayLabel } from "./core.js";

let pads = [];
let current = null; // id
let saveTimer = null;

export async function load() {
  pads = await call("pads");
  current = pads[0]?.id ?? null;
}

const titleOf = (p) => (p.body.trim().split("\n")[0] || "Empty pad").slice(0, 60);

async function create() {
  pads = await call("save_pad", { pad: { id: 0, body: "", updated: 0 } });
  current = pads[0].id;
  render();
  $("#pad-editor")?.focus();
}

function autosave(body) {
  const pad = pads.find((p) => p.id === current);
  if (!pad) return;
  pad.body = body;
  clearTimeout(saveTimer);
  $("#pad-status").textContent = "Saving…";
  saveTimer = setTimeout(async () => {
    pads = await call("save_pad", { pad });
    $("#pad-status").textContent = "Saved";
    document.querySelectorAll(".pad-row b").forEach((b) => { if (b.dataset.id == current) b.textContent = titleOf(pad); });
  }, 600);
}

export function render() {
  const pad = pads.find((p) => p.id === current);
  $("#page-scratchpad").replaceChildren(
    h("div", { class: "page-head" }, h("h1", {}, "Scratchpad"), h("button", { class: "btn primary", onclick: create }, "New pad")),
    pads.length ? h("div", { class: "pad-layout" },
      h("div", { class: "pad-list" }, pads.map((p) => h("button", { class: "note-row pad-row", "aria-current": String(p.id === current),
        onclick: () => { current = p.id; render(); } },
        h("span", {}, h("b", { dir: "auto", "data-id": p.id }, titleOf(p)), h("small", {}, `${dayLabel(p.updated, "short")} · ${timeOf(p.updated)}`))))),
      pad ? h("div", { class: "card pad-editor" },
        h("textarea", { id: "pad-editor", dir: "auto", placeholder: `Click here and hold ${state.boot.keys.talk} to dictate…`,
          oninput: (e) => autosave(e.target.value) }, pad.body),
        h("div", { class: "pad-tools" }, h("span", { class: "muted", id: "pad-status" }, "Saved"), h("span", { class: "grow" }),
          h("button", { class: "btn ghost", onclick: () => copyText($("#pad-editor").value) }, "Copy"),
          iconBtn("trash", "Delete pad", async () => {
            if (!confirm("Delete this pad?")) return;
            pads = await call("delete_pad", { id: pad.id });
            current = pads[0]?.id ?? null;
            render();
          }))) : null)
      : h("div", { class: "card empty" }, h("p", {}, "A place for drafts. Dictate a thought, an email or a to-do list, then copy it wherever you need it."),
        h("button", { class: "btn primary", onclick: create }, "Start a pad")),
  );
}
