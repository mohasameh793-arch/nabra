// Notetaker: start/stop call notes, live They/You transcript, past notes, overview panel, ask your notes.
import { $, h, ICONS, iconBtn, call, copyText, byDay, timeOf, clock, markdown, languageSelect, state, toast, notify, listen } from "./core.js";
import { people, whoCell, whoLabel, peopleBar } from "./speakers.js";
import { openSettings } from "./settings.js";

let events = [];
let mcpClosed = (() => { try { return localStorage.getItem("mcp-banner-closed") === "1"; } catch { return false; } })();

const ui = {
  cards: [],
  selected: null,   // full note
  live: null,       // { id, started, lines }
  consent: false,
  summarizing: false,
  asking: false,
  answer: null,
  query: "",
};
let ticker = null;

export async function load() {
  events = await call("calendar_events");
  ui.cards = await call("notes");
  if (!ui.selected && ui.cards.length) await select(ui.cards[0].id, false);
}

export function startLive(meeting, lines = []) {
  ui.live = { id: meeting.id, started: Date.now() - (meeting.elapsed ?? 0) * 1000, lines, ppl: people(null) };
  call("note_speakers", { id: meeting.id }).then((j) => { if (ui.live?.id === meeting.id) ui.live.ppl = people(j); }).catch(() => {});
}

/** From "Ask my meetings" → Open: show that note with its transcript open at the cited moment. */
export async function openNote(id, t) {
  await select(id, false);
  render();
  const full = document.querySelector("#page-notetaker details.full");
  if (!full) return;
  full.open = true;
  const rows = [...full.querySelectorAll("li[data-t]")];
  const row = rows.reduce((best, r) => (Math.abs(r.dataset.t - t) < Math.abs((best?.dataset.t ?? Infinity) - t) ? r : best), null);
  row?.scrollIntoView({ block: "center" });
  row?.classList.add("flash");
}

async function select(id, rerender = true) {
  ui.selected = await call("note", { id });
  if (rerender) render();
}

async function toggle() {
  if (!ui.live && !state.settings.notes_consent) {
    ui.consent = true;
    return render();
  }
  await call("toggle_meeting");
}

export function render() {
  const page = $("#page-notetaker");
  clearInterval(ticker);
  const startBtn = ui.live
    ? h("button", { class: "btn live", onclick: toggle, html: ICONS.stop }, h("span", { id: "live-clock" }, clock((Date.now() - ui.live.started) / 1000)))
    : h("button", { class: "btn soft", onclick: toggle, html: ICONS.record }, "Start Notetaker");
  if (ui.live) ticker = setInterval(() => { const c = $("#live-clock"); if (c) c.textContent = clock((Date.now() - ui.live.started) / 1000); }, 500);

  const main = h("div", { class: "notes-main" },
    h("div", { class: "page-head" }, h("h1", {}, "Notetaker"),
      h("div", { class: "row" }, iconBtn("gear", "Notes settings", () => openSettings("notes")), startBtn)),
    mcpClosed ? null : h("div", { class: "card mcp-banner" },
      h("span", { class: "doc", html: ICONS.send }),
      h("div", { style: "flex:1" }, h("b", {}, "Connect your AI tools to your notes"),
        h("p", {}, "Let Claude, ChatGPT and other MCP apps search and read your call notes. Read-only and local.")),
      h("button", { class: "btn ghost", onclick: () => { mcpClosed = true; try { localStorage.setItem("mcp-banner-closed", "1"); } catch {} render(); } }, "Skip"),
      h("button", { class: "btn", onclick: () => openSettings("connections") }, "Set up")),
    todayCard(),
    ui.consent ? h("div", { class: "card consent" },
      h("p", {}, "Let everyone on the call know you're transcribing it. Recording rules differ by country. ",
        h("span", { class: "muted" }, "Headphones give the cleanest They / You split.")),
      h("div", { class: "row" },
        h("button", { class: "btn", onclick: () => { ui.consent = false; render(); } }, "Cancel"),
        h("button", { class: "btn primary", onclick: async () => {
          await call("accept_notes_consent");
          state.settings.notes_consent = true;
          ui.consent = false;
          await call("toggle_meeting");
        } }, "Got it, start"))) : null,
    ui.live ? h("div", { class: "card live-card" },
      h("div", { class: "label" }, h("span", { class: "live-dot" }), "Live transcript"),
      peopleBar(ui.live.ppl, ui.live.id, true),
      liveLines().length ? transcript(liveLines(), ui.live.ppl, ui.live.id) : h("p", { class: "muted" }, "Words appear as people speak.")) : null,
    h("div", { class: "tabs" }, h("button", { "aria-selected": "true" }, "Past notes"), h("span", { class: "grow" }),
      h("input", { type: "search", placeholder: "Search notes", value: ui.query, style: "width:200px;padding:6px 10px",
        "aria-label": "Search notes", oninput: (e) => { ui.query = e.target.value; render(); $("#page-notetaker input[type=search]")?.focus(); } })),
    pastNotes(),
  );
  page.replaceChildren(h("div", { class: "notes-layout" }, main, panel()));
}

function todayCard() {
  if (!state.boot.calendar_connected) {
    return h("div", { class: "today-card" }, h("div", { class: "label" }, "Today"),
      h("div", { class: "empty-cal", html: '<svg viewBox="0 0 24 24"><rect x="4" y="5" width="16" height="15" rx="2"/><path d="M4 10h16M8.5 3v4M15.5 3v4"/></svg>' },
        h("span", { style: "font-size:17px" }, "No meetings found"),
        h("button", { class: "btn primary", onclick: () => openSettings("calendar") }, "Connect calendar")));
  }
  const now = Date.now();
  const end = new Date(); end.setHours(23, 59, 59, 999);
  const today = events.filter((e) => e.end > now - 3600_000 && e.start <= end.getTime() && !e.all_day);
  return h("div", { class: "today-card" }, h("div", { class: "label", style: "margin-bottom:6px" }, "Today"),
    today.length ? today.map((e) => {
      const live = e.start <= now && e.end > now;
      return h("div", { class: `meet-row${live ? " now" : ""}` },
        h("time", {}, live ? "Now" : `${timeOf(e.start)} – ${timeOf(e.end)}`), h("b", { dir: "auto" }, e.title),
        ui.live ? null : h("button", { class: live ? "btn primary" : "btn soft", onclick: async () => {
          if (!state.settings.notes_consent) { ui.consent = true; return render(); }
          await call("start_notes_for", { title: e.title });
        } }, "Take notes"));
    }) : h("div", { class: "empty-cal" }, "No more meetings today"));
}

function pastNotes() {
  const q = ui.query.trim().toLowerCase();
  const cards = q ? ui.cards.filter((c) => c.title.toLowerCase().includes(q)) : ui.cards;
  if (!cards.length) {
    return h("p", { class: "empty" }, ui.cards.length ? "No notes match." :
      `No notes yet. Press Start Notetaker or ${state.boot.keys.notes} when a call begins.`);
  }
  return byDay(cards, (c) => c.started_at, "short").map(([label, items]) => h("div", {},
    h("div", { class: "day" }, h("span", { class: "label" }, label)),
    items.map((c) => h("button", { class: "note-row", "aria-current": String(ui.selected?.id === c.id), onclick: () => select(c.id) },
      h("span", { class: "doc", html: ICONS.doc }),
      h("span", {}, h("b", { dir: "auto" }, c.title), h("small", {}, `${timeOf(c.started_at)} · ${clock(c.seconds)}`))))));
}

// The live call's finished lines plus what each side is saying right now.
const liveLines = () => [...ui.live.lines, ...Object.values(ui.live.partials ?? {})].sort((a, b) => a.t - b.t);

function transcript(lines, ppl = people(null), noteId = null) {
  return h("ol", { class: "transcript" }, lines.map((l) => h("li", { class: l.partial ? `${l.who} partial` : l.who, "data-t": l.t },
    whoCell(l, ppl, noteId, h("time", {}, clock(l.t))),
    h("div", { class: "said", dir: "auto" }, l.text))));
}

function panel() {
  const n = ui.selected;
  if (!n) return h("aside", { class: "notes-panel" }, h("p", { class: "muted" }, "Pick a note to see its overview."));
  const lang = languageSelect(state.settings.summary_language);
  const summarize = h("button", { class: "btn primary", disabled: ui.summarizing, onclick: async () => {
    ui.summarizing = true;
    render();
    try {
      ui.selected = await call("summarize_note", { id: n.id, language: lang.value || null });
      ui.cards = await call("notes");
    } finally {
      ui.summarizing = false;
      render();
    }
  } }, ui.summarizing ? "Summarizing…" : n.summary ? "Summarize again" : "Summarize");
  const ppl = people(n);
  const text = n.lines.map((l) => `[${clock(l.t)}] ${whoLabel(l, ppl)}: ${l.text}`).join("\n");
  return h("aside", { class: "notes-panel" },
    h("h2", { dir: "auto" }, n.title || "Untitled call"),
    h("div", { class: "meta" }, `${new Date(n.started_at).toLocaleDateString(undefined, { weekday: "long", month: "short", day: "numeric" })} • ${timeOf(n.started_at)}`),
    h("div", { class: "label" }, "Summary"),
    n.summary ? h("div", { class: "overview", dir: "auto", html: markdown(n.summary) })
      : h("p", { class: "muted" }, ui.summarizing ? "Your local AI is reading the whole transcript…" : n.lines.length ? "No summary yet. Click Summarize." : "Nothing was said in this call."),
    n.thoughts?.trim() ? [h("div", { class: "label", style: "margin-top:18px" }, "My thoughts"),
      h("div", { class: "overview", dir: "auto", style: "white-space:pre-wrap" }, n.thoughts)] : null,
    h("div", { class: "panel-tools" }, lang, summarize,
      n.summary ? h("button", { class: "btn", onclick: () => copyText(n.summary, "Summary copied") }, "Copy summary") : null,
      h("button", { class: "btn", onclick: () => copyText(text, "Transcript copied") }, "Copy transcript"),
      h("button", { class: "btn danger", onclick: async () => {
        if (!confirm("Delete this note and its transcript? This can't be undone.")) return;
        await call("delete_note", { id: n.id });
        ui.selected = null;
        await load();
        render();
      } }, "Delete")),
    n.lines.length ? h("details", { class: "full" }, h("summary", {}, `Transcript · ${n.lines.length} lines`), transcript(n.lines, ppl, n.id)) : null);
}

// ---------- live events ----------
export function wire() {
  listen("meeting", async (e) => {
    const m = e.payload;
    $("#nav-live").hidden = !m.active;
    if (m.active) startLive(m);
    else ui.live = null;
    if (!$("#page-notetaker").hidden) render();
  });
  const live = (p) => ui.live && p.note === ui.live.id; // ignore a previous meeting still finishing
  listen("note-partial", (e) => {
    if (!live(e.payload)) return;
    const p = e.payload;
    ui.live.partials ??= {};
    if (p.text) ui.live.partials[p.who] = { ...p, partial: true };
    else delete ui.live.partials[p.who];
    if (!$("#page-notetaker").hidden) render();
  });
  listen("note-line", (e) => {
    if (!live(e.payload)) return;
    if (ui.live.partials) delete ui.live.partials[e.payload.who];
    ui.live.lines.push(e.payload);
    ui.live.lines.sort((a, b) => a.t - b.t);
    if (!$("#page-notetaker").hidden) render();
  });
  listen("note-speakers", async (e) => {
    const id = e.payload.id;
    if (ui.live?.id === id) ui.live.ppl = people(e.payload);
    if (ui.selected?.id === id) ui.selected = await call("note", { id });
    if (!$("#page-notetaker").hidden) render();
  });
  listen("note-drop", (e) => {
    if (!live(e.payload)) return;
    ui.live.lines = ui.live.lines.filter((l) => l.id !== e.payload.id);
    if (!$("#page-notetaker").hidden) render();
  });
  listen("note-saved", async (e) => {
    ui.cards = await call("notes");
    await select(e.payload, false);
    notify("Note saved", "Open it and click Summarize when you're ready");
    if (!$("#page-notetaker").hidden) render();
  });
  listen("note-updated", async (e) => {
    ui.cards = await call("notes");
    if (ui.selected?.id === e.payload) {
      ui.summarizing = false;
      await select(e.payload, false);
    }
    const card = ui.cards.find((c) => c.id === e.payload);
    notify("Overview ready", card?.title ?? "");
    if (!$("#page-notetaker").hidden) render();
  });
  listen("notes-problem", (e) => {
    ui.summarizing = false;
    toast(String(e.payload));
    notify("Notetaker problem", String(e.payload));
    if (!$("#page-notetaker").hidden) render();
  });
  listen("calendar", async () => {
    events = await call("calendar_events");
    state.boot.calendar_connected = true;
    if (!$("#page-notetaker").hidden) render();
  });
  listen("consent-needed", () => {
    ui.consent = true;
    render();
  });
}
