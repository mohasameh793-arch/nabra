// Settings dialog (left nav, sections) and the Help dialog.
import { $, h, iconBtn, call, state, toast, copyText, LANGUAGES, languageName, languageSelect, getTheme, setTheme } from "./core.js";

const SECTIONS = [["general", "General"], ["languages", "Languages"], ["notes", "Notes"], ["calendar", "Calendar"],
  ["connections", "Connections"], ["privacy", "Privacy"], ["about", "About"]];
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
      const theme = h("div", { class: "seg" }, [["light", "Light"], ["dark", "Dark"], ["system", "Windows"]].map(([v, name]) =>
        h("button", { "aria-pressed": String(getTheme() === v), onclick: () => { setTheme(v); draw(); } }, name)));
      return [h("h2", {}, "General"),
        field("Your name", "Used for the greeting.", h("input", { type: "text", value: s.name, onchange: (e) => save({ name: e.target.value.trim() }) })),
        field("Appearance", null, theme),
        field("Start with Windows", "Open Nabra in the background when you sign in, so your shortcuts always work.", (() => {
          const t = h("input", { type: "checkbox", class: "switch", "aria-label": "Start with Windows", onchange: async (e) => {
            state.boot.autostart = await call("set_autostart", { on: e.target.checked });
            e.target.checked = state.boot.autostart;
            toast(state.boot.autostart ? "Nabra will start with Windows" : "Nabra won't start with Windows");
          } });
          t.checked = state.boot.autostart;
          return t;
        })()),
        field("Dictate", "Hold to talk, release to insert. Or click the mic on the pill for hands-free.", h("span", {}, h("kbd", {}, state.boot.keys.talk))),
        field("Note taking", "Start or stop call notes from anywhere.", h("span", {}, h("kbd", {}, state.boot.keys.notes))),
        field("Commands & transforms", "Hold and speak: \"make it shorter\", \"scratch that\", \"new line\". Works on selected text or your last dictation.", h("span", {}, h("kbd", {}, state.boot.keys.command))),
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
        field("Meeting reminders", "When a calendar meeting starts, the pill offers to take notes.", (() => {
          const t = h("input", { type: "checkbox", class: "switch", "aria-label": "Meeting reminders", onchange: (e) => save({ meeting_prompts: e.target.checked }) });
          t.checked = s.meeting_prompts;
          return t;
        })()),
        field("Who is who", "Computer audio (Zoom, Meet, Teams, Discord) is \"They\". Your microphone is \"You\". Headphones keep them apart.", h("span")),
        field("Privacy", "Call audio is transcribed on this PC and never saved. Only the text is kept.", h("span"))];
    }
    case "calendar": {
      const connected = state.boot.calendar_connected;
      const url = h("input", { type: "text", placeholder: "https://calendar.google.com/calendar/ical/…/basic.ics", "aria-label": "iCal link" });
      return [h("h2", {}, "Calendar"),
        h("p", { class: "muted" }, "See today's meetings in Notetaker and get a nudge to take notes when one starts. ",
          "Nabra reads your calendar's private iCal link: no sign-in, and the link is kept in Windows Credential Manager."),
        connected
          ? h("div", { class: "field" }, h("div", {}, h("b", {}, "Connected"), h("p", {}, "Refreshed every 10 minutes.")),
            h("div", { class: "stack" },
              h("button", { class: "btn", onclick: async () => { const n = await call("refresh_calendar"); toast(`${n} upcoming events`); } }, "Refresh now"),
              h("button", { class: "btn danger", onclick: async () => { await call("disconnect_calendar"); state.boot.calendar_connected = false; draw(); document.dispatchEvent(new CustomEvent("settings-changed")); } }, "Disconnect")))
          : h("div", { class: "field wide" }, h("div", { class: "stack" }, url,
            h("button", { class: "btn primary", style: "justify-self:start", onclick: async (e) => {
              e.target.disabled = true;
              try {
                const n = await call("connect_calendar", { url: url.value });
                state.boot.calendar_connected = true;
                toast(`Calendar connected · ${n} upcoming events`);
                draw();
                document.dispatchEvent(new CustomEvent("settings-changed"));
              } finally { e.target.disabled = false; }
            } }, "Connect"))),
        h("div", { class: "field wide" }, h("div", {}, h("b", {}, "Where to find the link"),
          h("p", {}, "Google Calendar: Settings → your calendar → Integrate calendar → Secret address in iCal format."),
          h("p", {}, "Outlook: Settings → Calendar → Shared calendars → Publish a calendar → ICS link.")))];
    }
    case "connections": {
      const setup = await call("mcp_setup");
      return [h("h2", {}, "Connections"),
        h("p", { class: "muted" }, "Let AI tools that support MCP (Claude, ChatGPT desktop, Cursor and others) search and read your call notes. ",
          "The connection is read-only and runs on this PC."),
        h("div", { class: "field wide" }, h("div", { class: "stack" }, h("b", {}, "Claude Code"),
          h("div", { class: "code" }, setup.claude_code),
          h("button", { class: "btn", style: "justify-self:start", onclick: () => copyText(setup.claude_code, "Command copied") }, "Copy command"))),
        h("div", { class: "field wide" }, h("div", { class: "stack" }, h("b", {}, "Claude Desktop and other MCP apps"),
          h("p", {}, "Add this to the app's MCP configuration (for Claude Desktop: Settings → Developer → Edit config)."),
          h("div", { class: "code" }, JSON.stringify(setup.json, null, 2)),
          h("button", { class: "btn", style: "justify-self:start", onclick: () => copyText(JSON.stringify(setup.json, null, 2), "Configuration copied") }, "Copy configuration")))];
    }
    case "privacy": {
      const keep = h("input", { type: "checkbox", class: "switch", "aria-label": "Keep dictation audio", onchange: (e) =>
        save({ keep_clips: e.target.checked }, "Saved. Applies the next time Nabra starts.") });
      keep.checked = s.keep_clips;
      return [h("h2", {}, "Privacy"),
        h("p", { class: "muted" }, "Speech recognition and AI cleanup run on this PC. Nothing you say is uploaded."),
        field("Keep my dictation audio for accuracy testing", "Saves your own dictations (never call audio) to the clips folder in Nabra's data folder, so recognition can be measured on your voice.", keep),
        field("Delete all dictation history", "Removes every saved dictation. Notes and dictionary stay.",
          h("button", { class: "btn danger", onclick: async () => {
            if (!confirm("Delete all dictation history? This can't be undone.")) return;
            await call("clear_history");
            state.history = [];
            document.dispatchEvent(new CustomEvent("history-cleared"));
            toast("History deleted");
          } }, "Delete all")),
        field("Forget saved voices", "Call notes remember the voices you name (as numbers, never audio) to label them in later calls. This forgets them all; your notes keep their names.",
          h("button", { class: "btn danger", onclick: async () => {
            if (!confirm("Forget every saved voice? Speakers will show as Speaker 1, 2… until you name them again.")) return;
            await call("forget_voices");
            toast("Saved voices forgotten");
          } }, "Forget voices"))];
    }
    case "about": {
      const e = state.boot.engine;
      return [h("h2", {}, "About Nabra"),
        field("Version", null, h("span", {}, state.boot.version)),
        field("Check for updates automatically", "Nabra checks every few hours and tells you when a new version is ready. Nothing installs until you click Update. Updates are signed, so only official releases install.", (() => {
          const t = h("input", { type: "checkbox", class: "switch", "aria-label": "Check for updates automatically",
            onchange: (ev) => save({ auto_update: ev.target.checked }, ev.target.checked ? "Update checks on" : "Update checks off") });
          t.checked = s.auto_update;
          return t;
        })()),
        field("Updates", null, h("button", { class: "btn", onclick: async (ev) => {
          ev.target.disabled = true;
          ev.target.textContent = "Checking…";
          try {
            const r = await call("check_updates");
            toast(r.status === "up_to_date" ? `You're on the latest version (${r.version})` : `Nabra ${r.version} is available`);
          } finally {
            ev.target.disabled = false;
            ev.target.textContent = "Check for updates";
          }
        } }, "Check for updates")),
        field("Speech engine", "Whisper large-v3, local", h("span", {}, e ? (e.device === "cuda" ? "Running on GPU" : "Running on CPU") : "Starting…")),
        field("AI cleanup & overviews", "Qwen3 8B, local", h("span", {}, e?.llm ? "Ready" : "Off")),
        field("Models", "Speech and AI models downloaded during setup.",
          h("button", { class: "btn", onclick: () => { $("#settings-modal").close(); import("../hub.js").then((m) => m.go("setup")); } }, "Open setup")),
        field("Your data", "Settings, dictionary, history and notes stay in this PC's app data folder.", h("span")),
        field("Open source", "Nabra is MIT-licensed. An independent project, not affiliated with Wispr.", h("span"))];
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
      h("li", {}, "Hold ", h("kbd", {}, state.boot.keys.command), " and say \"make it shorter\" or \"scratch that\" to edit text."),
      h("li", {}, "Say a snippet's trigger to type its saved text. Say \"new line\" for a line break."),
      h("li", {}, "Text didn't appear in an admin window? Use the tray icon → Copy last dictation.")),
    h("div", { class: "row" }, h("button", { class: "btn primary", onclick: () => modal.close() }, "Got it"))));
  modal.showModal();
}
