// The pill never takes focus (the window is non-activating), so clicking here leaves the cursor in the
// app you were typing in. Rust decides which view to show; this page only draws it.
const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);

const BARS = 9;
$("levels").innerHTML = "<i></i>".repeat(BARS);
const bars = [...$("levels").children];
const history = new Array(BARS).fill(0);

const clock = (s) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, "0")}`;

function render(v) {
  document.body.dataset.view = v.view;
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
      $("clock").textContent = clock(v.seconds);
      $("msg").textContent = v.command ? "Say a command, then release" : v.hands_free ? "Click ■ to insert" : "Release to insert";
      $("stop").hidden = !v.hands_free;
      $("stop").dataset.action = "mic";
      history.shift();
      history.push(Math.min(1, v.level * 14)); // speech RMS is small; scale to 0..1
      bars.forEach((b, i) => (b.style.height = `${4 + history[i] * 20}px`));
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
  leaveTimer = setTimeout(() => {
    hovering = false;
    invoke("pill_hover", { on: false });
  }, 300);
});

for (const btn of [$("mic"), $("notes"), $("chev")]) {
  btn.addEventListener("mouseenter", () => $(btn.dataset.tip).classList.add("show"));
  btn.addEventListener("mouseleave", () => $(btn.dataset.tip).classList.remove("show"));
}
$("mic").addEventListener("click", () => invoke("pill_mic"));
$("notes").addEventListener("click", () => invoke("pill_notes"));

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
$("stop").addEventListener("click", () => invoke($("stop").dataset.action === "notes" ? "pill_notes" : "pill_mic"));
// Double-click the mic button to open the Nabra window.
$("mic").addEventListener("dblclick", () => invoke("open_hub", { page: null }));

window.__TAURI__.event.listen("pill", (e) => render(e.payload));
render({ view: "idle" });
