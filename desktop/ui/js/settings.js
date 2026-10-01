// Settings dialog (left nav, sections) and the Help dialog.
import { $, h, iconBtn, call, state, toast, LANGUAGES, languageName, languageSelect } from "./core.js";

const SECTIONS = [["general", "General"], ["languages", "Languages"], ["notes", "Notes"], ["privacy", "Privacy"], ["about", "About"]];
let section = "general";

async function save(patch, message = "Saved") {
  const next = { ...state.settings, ...patch };
  await call("save_settings", { settings: next });
  state.settings = next;
  document.dispatchEvent(new CustomEvent("settings-changed"));
  toast(message);
}

const field = (title, help, control) =>
  h("div", { class: "field" }, h("div", {}, h("b", {}, title), help ? h("p", {}, help) : null), control);

async function body() {
  const s = state.settings;
  switch (section) {
    case "general": {
      const mics = await call("microphones");
      const mic = h("select", { "aria-label": "Microphone", onchange: (e) => save({ microphone: e.target.value || null }, "Microphone saved") },
        h("option", { value: "" }, "Windows default"), mics.map((m) => h("option", { value: m }, m)));
      mic.value = s.microphone ?? "";
      const mode = h("div", { class: "seg" }, [["clean", "Clean"], ["raw", "Raw"]].map(([v, name]) =>
        h("button", { "aria-pressed": String(s.mode === v), onclick: async () => { await save({ mode: v }, `${name} mode`); draw(); } }, name)));
      return [h("h2", {}, "General"),
        field("Your name", "Used for the greeting.", h("input", { type: "text", value: s.name, onchange: (e) => save({ name: e.target.value.trim() }) })),
        field("Dictate", "Hold to talk, release to insert. Or click the mic on the pill for hands-free.", h("span", {}, h("kbd", {}, state.boot.keys.talk))),
        field("Note taking", "Start or stop call notes from anywhere.", h("span", {}, h("kbd", {}, state.boot.keys.notes))),
        field("Microphone", null, mic),
        field("Writing", "Clean fixes terms, punctuation and fillers with the local AI. Raw keeps exactly what was heard.", mode)];
    }
    case "languages":
      return [h("h2", {}, "Languages"),
        h("p", { class: "muted" }, "Tick the languages you speak. Nabra picks among these, even when you switch mid-sentence, and it never translates."),
        h("div", { class: "lang-chips", style: "margin-top:16px" }, LANGUAGES.map(([code, en, native]) =>
          h("button", { class: "lang-chip", "aria-pressed": String(s.languages.includes(code)), onclick: async () => {
            const langs = s.languages.includes(code) ? s.languages.filter((c) => c !== code) : [...s.languages, code];
            if (!langs.length) return toast("Keep at least one language");
            await save({ languages: langs }, `Listening for ${langs.map(languageName).join(", ")}`);
            draw();
          } }, en, en !== native ? h("small", {}, native) : null)))];
    case "notes": {
      const sel = languageSelect(s.summary_language, "Same as the call");
      sel.addEventListener("change", () => save({ summary_language: sel.value || null }, "Summary language saved"));
      return [h("h2", {}, "Notes"),
        field("Overview language", "Language for call overviews. You can also change it per note.", sel),
        field("Who is who", "Computer audio (Zoom, Meet, Teams, Discord) is \"They\". Your microphone is \"You\". Headphones keep them apart.", h("span")),
        field("Privacy", "Call audio is transcribed on this PC and never saved. Only the text is kept.", h("span"))];
    }
    case "privacy": {
      const keep = h("input", { type: "checkbox", class: "switch", "aria-label": "Keep dictation audio", onchange: (e) =>
        save({ keep_clips: e.target.checked }, "Saved. Applies the next time Nabra starts.") });
      keep.checked = s.keep_clips;
      return [h("h2", {}, "Privacy"),
        h("p", { class: "muted" }, "Speech recognition and AI cleanup run on this PC. Nothing you say is uploaded."),
        field("Keep my dictation audio for accuracy testing", "Saves your own dictations (never call audio) to bench\\real so recognition can be measured on your voice.", keep),
        field("Delete all dictation history", "Removes every saved dictation. Notes and dictionary stay.",
          h("button", { class: "btn danger", onclick: async () => {
            if (!confirm("Delete all dictation history? This can't be undone.")) return;
            await call("clear_history");
            state.history = [];
            document.dispatchEvent(new CustomEvent("history-cleared"));
            toast("History deleted");
          } }, "Delete all"))];
    }
    case "about": {
      const e = state.boot.engine;
      return [h("h2", {}, "About Nabra"),
        field("Version", null, h("span", {}, state.boot.version)),
        field("Speech engine", "Whisper large-v3, local", h("span", {}, e ? (e.device === "cuda" ? "Running on GPU" : "Running on CPU") : "Starting…")),
        field("AI cleanup & overviews", "Qwen3 8B, local", h("span", {}, e?.llm ? "Ready" : "Off")),
        field("Your data", "Settings, dictionary, history and notes stay in this PC's app data folder.", h("span"))];
    }
  }
  return [];
}

async function draw() {
  const modal = $("#settings-modal");
  const content = await body();
  modal.replaceChildren(h("div", { class: "settings" },
    h("nav", {}, h("h3", {}, "Settings"), SECTIONS.map(([id, name]) =>
      h("button", { class: "nav", "aria-current": section === id ? "page" : null, onclick: () => { section = id; draw(); } }, h("span", {}, name)))),
    h("div", { class: "body" }, iconBtn("close", "Close", () => modal.close(), { class: "icon-btn modal-close" }), ...content)));
}

export async function openSettings(which = "general") {
  section = which;
  await draw();
  const modal = $("#settings-modal");
  if (!modal.open) modal.showModal();
}

export function openHelp() {
  const modal = $("#help-modal");
  modal.replaceChildren(h("div", { class: "form" },
    h("h2", {}, "Using Nabra"),
    h("ul", { class: "help-list" },
      h("li", {}, "Hold ", h("kbd", {}, state.boot.keys.talk), ", talk, release. The text appears where your cursor is."),
      h("li", {}, "Hover the pill on the right edge for the mic (hands-free) and note-taking buttons."),
      h("li", {}, h("kbd", {}, state.boot.keys.notes), " starts or stops call notes. The overview is written when you stop."),
      h("li", {}, "Mix languages freely. Pick yours in Settings → Languages."),
      h("li", {}, "A word keeps coming out wrong? Add it to the Dictionary."),
      h("li", {}, "Text didn't appear in an admin window? Use the tray icon → Copy last dictation.")),
    h("div", { class: "row" }, h("button", { class: "btn primary", onclick: () => modal.close() }, "Got it"))));
  modal.showModal();
}
