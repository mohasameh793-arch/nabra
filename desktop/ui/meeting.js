// The meeting window: header with timer, sound meters, Stop; tabs My thoughts · Transcript · Summary.
import { $, h, call, listen, clock, timeOf, markdown, languageSelect, copyText, toast, getTheme, setTheme, state } from "./js/core.js";

setTheme(getTheme());
const win = window.__TAURI__.window.getCurrentWindow();
$("#mw-min").addEventListener("click", () => win.minimize());
$("#mw-close").addEventListener("click", () => call("close_meeting_window"));

const note = { id: null, started: 0, title: "", live: false, lines: [], summary: null, seconds: 0 };
let ticker = null;
let saveTimer = null;
let summarizing = false;

// ---------- header ----------
function drawHead() {
  $("#mw-title").textContent = note.title || "New note";
  const d = new Date(note.started || Date.now());
  $("#mw-meta").textContent = `${d.toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" })} · started ${timeOf(d.getTime())}`;
  $("#mw-live").classList.toggle("stopped", !note.live);
  $("#mw-state").textContent = note.live ? "Recording" : "Stopped";
  clearInterval(ticker);
  const tick = () => ($("#mw-clock").textContent = clock(note.live ? (Date.now() - note.started) / 1000 : note.seconds));
  tick();
  if (note.live) ticker = setInterval(tick, 500);
}

$("#mw-stop").addEventListener("click", async () => {
  $("#mw-stop").disabled = true;
  try {
    await call("toggle_meeting");
  } finally {
    $("#mw-stop").disabled = false;
  }
});

// ---------- tabs ----------
document.querySelectorAll(".mw-tabs button").forEach((b) => b.addEventListener("click", () => showTab(b.dataset.tab)));
function showTab(tab) {
  document.querySelectorAll(".mw-tabs button").forEach((b) => b.setAttribute("aria-selected", String(b.dataset.tab === tab)));
  document.querySelectorAll(".mw-panel").forEach((p) => (p.hidden = p.id !== `tab-${tab}`));
  if (tab === "summary") drawSummary();
  if (tab === "transcript") $("#tab-transcript").scrollTop = $("#tab-transcript").scrollHeight;
}

// ---------- my thoughts ----------
$("#mw-thoughts").addEventListener("input", () => {
  clearTimeout(saveTimer);
  $("#mw-saved").textContent = "Saving…";
  saveTimer = setTimeout(saveThoughts, 500);
});
async function saveThoughts() {
  if (!note.id) return;
  try {
    await call("save_thoughts", { id: note.id, text: $("#mw-thoughts").value });
    $("#mw-saved").textContent = "Saved";
  } catch {
    $("#mw-saved").textContent = "Will save when the note is stored";
  }
}

// ---------- transcript ----------
function lineEl(l) {
  return h("li", { class: l.who, "data-id": l.id },
    h("div", { class: "who" }, l.who === "you" ? "You" : "They", h("time", {}, clock(l.t))),
    h("div", { class: "said", dir: "auto" }, l.text));
}
function drawTranscript() {
  $("#mw-transcript").replaceChildren(...note.lines.map(lineEl));
  $("#mw-transcript-empty").hidden = note.lines.length > 0;
  $("#mw-count").textContent = note.lines.length ? `· ${note.lines.length}` : "";
}

// ---------- summary ----------
function drawSummary() {
  const panel = $("#tab-summary");
  if (note.live) {
    return panel.replaceChildren(h("div", { class: "mw-wait" }, h("b", {}, "Stop the note to summarize it"),
      h("span", {}, "The summary covers the whole transcript, so it's written after the meeting ends.")));
  }
  if (!note.lines.length) {
    return panel.replaceChildren(h("div", { class: "mw-wait" }, h("b", {}, "Nothing to summarize"), h("span", {}, "No one spoke during this note.")));
  }
  const lang = languageSelect(state.settings?.summary_language, "Same as the meeting");
  const run = h("button", { class: "btn primary", disabled: summarizing, onclick: async () => {
    summarizing = true;
    drawSummary();
    try {
      const saved = await call("summarize_note", { id: note.id, language: lang.value || null });
      note.summary = saved.summary;
      if (saved.title) note.title = saved.title;
      drawHead();
    } finally {
      summarizing = false;
      drawSummary();
    }
  } }, summarizing ? "Summarizing…" : note.summary ? "Summarize again" : "Summarize");
  panel.replaceChildren(
    h("div", { class: "mw-summary-tools" }, lang, run,
      note.summary ? h("button", { class: "btn", onclick: () => copyText(note.summary, "Summary copied") }, "Copy") : null),
    note.summary ? h("div", { class: "overview", dir: "auto", html: markdown(note.summary) })
      : h("div", { class: "mw-wait" }, h("b", {}, summarizing ? "Writing the summary…" : "Ready when you are"),
        h("span", {}, summarizing ? "Your local AI is reading the whole transcript." : "Click Summarize to get the key points, decisions and action items.")));
}

// ---------- loading ----------
async function loadLive(meeting) {
  Object.assign(note, { id: meeting.id, started: Date.now() - (meeting.elapsed ?? 0) * 1000, title: meeting.title ?? "",
    live: true, lines: await call("live_lines"), summary: null, seconds: 0 });
  $("#mw-thoughts").value = await call("live_thoughts");
  $("#mw-saved").textContent = "";
  drawHead();
  drawTranscript();
  drawSummary();
}

async function loadSaved(id) {
  const n = await call("note", { id });
  Object.assign(note, { id: n.id, started: n.started_at, title: n.title, live: false, lines: n.lines, summary: n.summary, seconds: n.seconds });
  if (document.activeElement !== $("#mw-thoughts")) $("#mw-thoughts").value = n.thoughts ?? "";
  drawHead();
  drawTranscript();
  drawSummary();
}

listen("meeting", async (e) => {
  if (e.payload.active) {
    await loadLive(e.payload);
    showTab("thoughts");
  } else {
    note.live = false;
    note.seconds = (Date.now() - note.started) / 1000;
    drawHead();
    drawSummary();
  }
});
listen("note-line", (e) => {
  if (!note.live) return;
  note.lines.push(e.payload);
  note.lines.sort((a, b) => a.t - b.t);
  const box = $("#tab-transcript");
  const atBottom = box.scrollHeight - box.scrollTop - box.clientHeight < 80;
  drawTranscript();
  if (atBottom) box.scrollTop = box.scrollHeight;
});
listen("note-drop", (e) => {
  note.lines = note.lines.filter((l) => l.id !== e.payload);
  drawTranscript();
});
listen("note-saved", async (e) => {
  if (e.payload !== note.id) return;
  await saveThoughts(); // anything typed between Stop and the save lands in the note now
  await loadSaved(e.payload);
});
listen("meeting-level", (e) => {
  const pct = (v) => `${Math.min(100, Math.round(v * 1400))}%`; // speech RMS is small; scale for display
  $("#lv-them").style.width = pct(e.payload.them);
  $("#lv-you").style.width = pct(e.payload.you);
});
listen("notes-problem", (e) => toast(String(e.payload)));

(async () => {
  const boot = await call("boot");
  state.boot = boot;
  state.settings = boot.settings;
  if (boot.meeting) await loadLive(boot.meeting);
  else {
    const latest = (await call("notes"))[0];
    if (latest) await loadSaved(latest.id);
    else drawHead();
  }
})().catch((e) => toast(`Couldn't load the note: ${e}`));
