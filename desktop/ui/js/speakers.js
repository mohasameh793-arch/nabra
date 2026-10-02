// Who said it, in call notes. Voices on the other side start as "Speaker 1, 2…"; click one to pick a name from
// the people in the meeting (calendar invite + meeting app) or type one. Nabra remembers that voice next time.
import { h, call, toast } from "./core.js";

/** {names: {s1: "Zaid"}, attendees: ["Ahmed", …]} from the backend's speakers JSON. */
export function people(json) {
  const names = {};
  for (const s of json?.speakers ?? []) names[s.id] = s.name;
  return { names, attendees: json?.attendees ?? [] };
}

export function whoLabel(line, ppl) {
  if (line.who === "you") return "You";
  if (!line.speaker) return "They";
  return ppl.names[line.speaker] || `Speaker ${line.speaker.replace(/^s/, "")}`;
}

/** The "who" cell of a transcript line: a button for voices that can be named. */
export function whoCell(line, ppl, noteId, time) {
  const label = whoLabel(line, ppl);
  const named = Boolean(ppl.names[line.speaker]);
  const who = line.speaker && !line.partial && noteId
    ? h("button", { class: named ? "who-name" : "who-name unnamed", title: named ? "Change name" : "Who is this?",
        onclick: (e) => { e.stopPropagation(); openNamer(e.currentTarget, noteId, line.speaker, ppl); } }, label)
    : label;
  return h("div", { class: "who" }, who, time);
}

let open = null;
function close() {
  open?.remove();
  open = null;
}
document.addEventListener("click", (e) => { if (open && !open.contains(e.target)) close(); });
document.addEventListener("keydown", (e) => { if (e.key === "Escape") close(); });

function openNamer(anchor, noteId, speaker, ppl) {
  close();
  const save = async (name) => {
    close();
    try {
      await call("name_speaker", { id: noteId, speaker, name });
      if (name) toast(`${name} it is. Nabra will recognise this voice next time.`);
    } catch (err) {
      toast(String(err));
    }
  };
  const input = h("input", { type: "text", placeholder: "Type a name", "aria-label": "Name", dir: "auto",
    onkeydown: (e) => { if (e.key === "Enter" && input.value.trim()) save(input.value.trim()); } });
  const taken = new Set(Object.entries(ppl.names).filter(([id, n]) => id !== speaker && n).map(([, n]) => n.toLowerCase()));
  const chips = ppl.attendees.filter((a) => !taken.has(a.toLowerCase()))
    .map((a) => h("button", { class: "chip", dir: "auto", onclick: () => save(a) }, a));
  open = h("div", { class: "namer", role: "dialog", "aria-label": "Who is this?" },
    h("b", {}, "Who is this?"),
    chips.length ? h("div", { class: "chips" }, chips) : h("small", { class: "muted" }, "No attendee names found yet. Type one:"),
    h("div", { class: "row" }, input, h("button", { class: "btn primary", onclick: () => input.value.trim() && save(input.value.trim()) }, "Save")),
    ppl.names[speaker] ? h("button", { class: "link", onclick: () => save("") }, "Not them? Clear the name") : null);
  document.body.append(open);
  const r = anchor.getBoundingClientRect();
  open.style.left = `${Math.max(8, Math.min(r.left, innerWidth - open.offsetWidth - 8))}px`;
  open.style.top = `${Math.min(r.bottom + 6, innerHeight - open.offsetHeight - 8)}px`;
  input.focus();
}

/** "In this meeting: Ahmed · Amjad · Zaid  [Find names] [+ Add]" (buttons only while recording). */
export function peopleBar(ppl, noteId, live) {
  const add = h("input", { class: "add-person", type: "text", placeholder: "+ Add name", "aria-label": "Add a name", dir: "auto",
    onkeydown: (e) => {
      if (e.key === "Enter" && add.value.trim()) call("add_attendees", { id: noteId, names: [add.value.trim()], scan: false });
    } });
  const scan = async (e) => {
    e.currentTarget.disabled = true;
    try {
      await call("add_attendees", { id: noteId, names: [], scan: true });
    } finally {
      e.currentTarget.disabled = false;
    }
  };
  const names = ppl.attendees.length ? ppl.attendees.map((a) => h("span", { class: "chip", dir: "auto" }, a))
    : [h("span", { class: "muted" }, live ? "Looking for names… open the participant list in Zoom, Teams or Meet, then Find names." : "No names")];
  return h("div", { class: "people-bar" }, h("small", { class: "muted" }, "In this meeting"), ...names,
    live ? h("button", { class: "link", onclick: scan, title: "Read the participant list of Zoom, Teams or Meet" }, "Find names") : null,
    live ? add : null);
}
