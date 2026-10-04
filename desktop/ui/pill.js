// The pill never takes focus (the window is non-activating), so clicking here leaves the cursor in the
// app you were typing in. Rust decides which view to show; this page only draws it.
const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);

// The capsule's voice meter: short lines whose length follows your voice, longest in the middle.
const LINES = 13;
$("lines").innerHTML = "<i></i>".repeat(LINES);
const lines = [...$("lines").children];
const recent = new Array(LINES).fill(0);
function meter(level) {
  recent.shift();
  recent.push(Math.min(1, level * 14)); // speech RMS is small; scale to 0..1
  const now = recent[LINES - 1];
  lines.forEach((l, i) => {
    const shape = 0.35 + 0.65 * Math.sin((Math.PI * (i + 0.5)) / LINES); // fuller in the middle
    const v = 0.15 + 0.85 * Math.max(now * shape, recent[(i * 3) % LINES] * shape * 0.8);
    l.style.setProperty("--len", `${Math.round(3 + v * 15)}px`);
  });
}

const clock = (s) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, "0")}`;

function render(v) {
  document.body.dataset.view = v.view;
  document.body.dataset.dock = v.dock || "right";
  $("chev").textContent = v.dock === "right" || !v.dock ? "‹" : "›";
  $("msg").textContent = "";
  $("detail").textContent = "";
  $("stop").hidden = true;
  $("take").hidden = true;
  $("dismiss").hidden = true;
  document.body.classList.toggle("command", v.view === "listening" && v.command);
  switch (v.view) {
    case "hover":
      $("key-talk").textContent = v.talk_key;
      $("key-notes").textContent = v.notes_key;
      $("notes").classList.toggle("on", v.notes_on);
      $("notes-label").textContent = v.notes_on ? "Stop notes" : "Start notes";
      $("notes").setAttribute("aria-label", $("notes-label").textContent);
      break;
    case "idle":
      closePanel();
      break;
    case "notes":
      $("clock").textContent = clock(v.seconds);
      $("msg").textContent = "Taking notes";
      $("stop").hidden = false;
      $("stop").dataset.action = "notes";
      break;
    case "listening":
      meter(v.level);
      $("wave").dataset.handsFree = v.hands_free ? "1" : "";
      $("wave").title = v.command ? "Say a command, then release" : v.hands_free ? "Click to insert" : "Release to insert";
      break;
    case "busy":
      recent.fill(0);
      break;
    case "reveal":
      $("reveal-title").textContent = v.title;
      $("reveal-text").textContent = v.text;
      $("reveal-open").hidden = !v.note;
      $("reveal-open").dataset.note = v.note || "";
      $("reveal-open").dataset.t = String(v.t || 0);
      $("reveal-copy").textContent = "Copy";
      break;
    case "meeting":
      $("msg").textContent = `${v.title} is starting`;
      $("take").textContent = "Take notes";
      $("take").dataset.action = "notes";
      $("take").hidden = false;
      $("dismiss").hidden = false;
      break;
    case "update":
      $("msg").textContent = `Nabra ${v.version} is available`;
      $("take").textContent = "Update";
      $("take").dataset.action = "update";
      $("take").hidden = false;
      $("dismiss").hidden = false;
      break;
    case "working":
      $("msg").textContent = v.label;
      break;
    case "result":
      $("msg").textContent = v.text;
      $("detail").textContent = v.detail;
      break;
    case "problem":
      $("msg").textContent = v.message;
      break;
  }
}

// Hover in → Rust grows the window and shows the buttons; leaving the window shrinks it back.
// Growing the window makes WebView2 fire a spurious "mouseleave", so leaving waits a moment and is
// cancelled by any movement still inside the pill.
let hovering = false;
let leaveTimer = null;
function enter() {
  clearTimeout(leaveTimer);
  if (!hovering) {
    hovering = true;
    invoke("pill_hover", { on: true });
  }
}
$("capsule").addEventListener("mouseenter", enter);
document.addEventListener("mousemove", (e) => {
  clearTimeout(leaveTimer);
  if (!hovering && e.target.closest("#capsule")) enter();
});
document.documentElement.addEventListener("mouseleave", () => {
  clearTimeout(leaveTimer);
  if (press?.dragging) return; // the window follows the mouse while dragging
  leaveTimer = setTimeout(() => {
    hovering = false;
    invoke("pill_hover", { on: false });
  }, 300);
});

for (const btn of [$("mic"), $("notes"), $("chev")]) {
  btn.addEventListener("mouseenter", () => $(btn.dataset.tip).classList.add("show"));
  btn.addEventListener("mouseleave", () => $(btn.dataset.tip).classList.remove("show"));
}
// Drag the pill (from the capsule or a button) to the left, bottom or right middle of any screen.
let press = null;
let dragged = false;
document.addEventListener("pointerdown", (e) => {
  if (!e.target.closest("#capsule, #mic, #notes, #wave") || e.button !== 0) return;
  press = { x: e.screenX, y: e.screenY, dragging: false };
  dragged = false;
  e.target.setPointerCapture?.(e.pointerId);
});
document.addEventListener("pointermove", (e) => {
  if (!press || press.dragging || Math.hypot(e.screenX - press.x, e.screenY - press.y) < 6) return;
  press.dragging = true;
  document.body.classList.add("dragging");
  invoke("pill_drag", { on: true });
});
function endPress() {
  if (press?.dragging) {
    dragged = true; // swallow the click that follows the drop
    document.body.classList.remove("dragging");
    invoke("pill_drag", { on: false });
  }
  press = null;
}
document.addEventListener("pointerup", endPress);
document.addEventListener("pointercancel", endPress); // touch/pen gave up: still end the drag
document.addEventListener("lostpointercapture", endPress);
const unlessDragged = (fn) => () => (dragged ? (dragged = false) : fn());

$("mic").addEventListener("click", unlessDragged(() => invoke("pill_mic")));
$("notes").addEventListener("click", unlessDragged(() => invoke("pill_notes")));
// Hands-free dictation: click the capsule to insert.
$("wave").addEventListener("click", unlessDragged(() => $("wave").dataset.handsFree && invoke("pill_mic")));

// › : today's remaining calendar meetings, or a prompt to connect a calendar.
const CAL_ICON = '<svg viewBox="0 0 24 24"><rect x="3.5" y="5" width="17" height="15" rx="2.5"/><path d="M3.5 10h17M8 3v4M16 3v4"/></svg>';
function closePanel() {
  $("panel").hidden = true;
  $("chev").classList.remove("open");
  document.body.classList.remove("panel-open");
}
$("chev").addEventListener("click", async () => {
  if (!$("panel").hidden) return closePanel();
  const [events, boot] = await Promise.all([invoke("calendar_events"), invoke("boot")]);
  const now = Date.now(), midnight = new Date().setHours(24, 0, 0, 0);
  const today = events.filter((e) => !e.all_day && e.end > now && e.start < midnight).slice(0, 4);
  const time = (ms) => new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  const esc = (t) => t.replace(/[&<>"]/g, (c) => `&#${c.charCodeAt(0)};`);
  $("panel").innerHTML = today.length
    ? `<h4>Upcoming meetings</h4><ul>${today.map((e) => `<li><time>${time(e.start)}</time><span>${esc(e.title)}</span></li>`).join("")}</ul>`
    : `${CAL_ICON}<h4>${boot.calendar_connected ? "You’re all done for today" : "No upcoming meetings"}</h4>` +
      (boot.calendar_connected ? "" : `<button id="cal-connect">Connect calendar</button>`);
  $("cal-connect")?.addEventListener("click", () => { closePanel(); invoke("open_hub", { page: "settings:calendar" }); });
  $("panel").hidden = false;
  $("chev").classList.add("open");
  document.body.classList.add("panel-open");
});
$("take").addEventListener("click", () => invoke($("take").dataset.action === "update" ? "install_update" : "pill_notes").catch(() => {}));
$("dismiss").addEventListener("click", () => invoke("pill_dismiss"));
$("reveal-close").addEventListener("click", () => invoke("pill_dismiss"));
$("reveal-copy").addEventListener("click", async () => {
  await invoke("copy_text", { text: $("reveal-text").textContent }).catch(() => {});
  $("reveal-copy").textContent = "Copied";
});
$("reveal-open").addEventListener("click", () => {
  const { note, t } = $("reveal-open").dataset;
  invoke("open_hub", { page: `note:${note}:${t}` });
  invoke("pill_dismiss");
});
$("stop").addEventListener("click", () => invoke($("stop").dataset.action === "notes" ? "pill_notes" : "pill_mic"));
// Double-click the mic button to open the Nabra window.
$("mic").addEventListener("dblclick", unlessDragged(() => invoke("open_hub", { page: null })));

window.__TAURI__.event.listen("pill", (e) => render(e.payload));
// Views sent before this page finished loading (e.g. "Starting Nabra…") were missed: ask for the current one.
invoke("pill_state").then((v) => render(v || { view: "idle" })).catch(() => render({ view: "idle" }));
