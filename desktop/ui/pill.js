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
      $("notes-label").textContent = v.notes_on ? "Stop note taking" : "Start note taking";
      $("notes").setAttribute("aria-label", $("notes-label").textContent);
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

for (const btn of [$("mic"), $("notes")]) {
  btn.addEventListener("mouseenter", () => $(btn.dataset.tip).classList.add("show"));
  btn.addEventListener("mouseleave", () => $(btn.dataset.tip).classList.remove("show"));
}
$("mic").addEventListener("click", () => invoke("pill_mic"));
$("notes").addEventListener("click", () => invoke("pill_notes"));
$("take").addEventListener("click", () => invoke("pill_notes"));
$("dismiss").addEventListener("click", () => invoke("pill_dismiss"));
$("stop").addEventListener("click", () => invoke($("stop").dataset.action === "notes" ? "pill_notes" : "pill_mic"));
// Double-click the capsule to open the Nabra window.
$("capsule").addEventListener("dblclick", () => invoke("open_hub", { page: null }));

window.__TAURI__.event.listen("pill", (e) => render(e.payload));
render({ view: "idle" });
