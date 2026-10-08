// First-run setup: download the speech/AI models this PC needs. Shown until everything is installed.
import { $, h, call, listen, state, toast } from "./core.js";

let status = null;
const progress = {}; // part → { done, total, stage }
let error = "";

export async function load() {
  status = await call("setup_status");
}

const gb = (mb) => `${(mb / 1000).toFixed(1)} GB`;

export function render() {
  if (!status) return;
  const missing = status.parts.filter((p) => !p.installed);
  const totalMb = missing.reduce((n, p) => n + p.mb, 0);
  const gpu = status.gpu_vram_mb;
  const ai = status.parts.some((p) => p.id === "qwen");
  const machine = status.npu
    ? `Intel NPU found. Nabra will run speech recognition on it (the first start takes a minute or two while the NPU prepares the model).${ai ? ` The NVIDIA GPU (${gb(gpu)}) runs the local AI model for cleanup, transforms and call overviews.` : " AI cleanup and call overviews need an NVIDIA GPU, so they stay off."}`
    : gpu
      ? `NVIDIA GPU with ${gb(gpu)} of memory found. Nabra will run Whisper large-v3 on it${ai ? ", plus a local AI model for cleanup, transforms and call overviews" : ". The local AI model needs about 10 GB of GPU memory, so AI cleanup stays off"}.`
      : "No NPU or NVIDIA GPU found. Nabra will run Whisper on your processor. It works, but each dictation takes a few seconds, and AI cleanup and call overviews stay off.";

  $("#page-setup").replaceChildren(h("div", { class: "narrow setup" },
    h("div", { class: "page-head" }, h("h1", {}, status.ready ? "Nabra is set up" : "Set up Nabra")),
    h("p", { class: "muted" }, "Nabra runs entirely on this PC. It needs its speech and AI models once; after that it works offline, and nothing you say is uploaded."),
    h("div", { class: "card setup-machine" }, h("b", {}, "Your PC"), h("p", {}, machine)),
    h("div", { class: "list" }, status.parts.map((p) => {
      const pr = progress[p.id];
      const pct = p.installed ? 100 : pr?.total ? Math.floor((pr.done / pr.total) * 100) : 0;
      return h("div", { class: "row-item setup-row" },
        h("div", {}, h("div", {}, p.label),
          h("div", { class: "bar-track" }, h("div", { class: "bar-fill", style: `width:${pct}%` }))),
        h("span", { class: "muted" }, p.installed ? "Installed" : pr ? `${pr.stage}${pr.total ? ` · ${pct}%` : "…"}` : `≈ ${gb(p.mb)}`));
    })),
    error ? h("div", { class: "card setup-error", role: "alert" }, h("b", {}, "Setup stopped"), h("p", {}, error),
      h("p", { class: "muted" }, "Your progress is kept. Try again to resume.")) : null,
    h("div", { class: "setup-actions" },
      status.ready
        ? h("button", { class: "btn primary", onclick: () => import("../hub.js").then((m) => m.go("dictation")) }, "Start using Nabra")
        : h("button", { class: "btn primary", disabled: status.running, onclick: run },
          status.running ? "Downloading…" : error ? "Try again" : `Download & set up (≈ ${gb(totalMb)})`),
      h("span", { class: "muted" }, `Saved in ${status.folder}`)),
  ));
}

async function run() {
  error = "";
  status.running = true;
  render();
  try {
    await call("setup_run");
  } catch (e) {
    error = String(e);
  }
  await load();
  render();
}

export function wire() {
  // e.g. not enough disk space for the local AI model: speech still installs, the user should know why.
  listen("setup-note", (e) => toast(e.payload));
  listen("setup-progress", (e) => {
    progress[e.payload.part] = e.payload;
    if (e.payload.stage === "Done") {
      const p = status?.parts.find((x) => x.id === e.payload.part);
      if (p) p.installed = true;
    }
    if (!$("#page-setup").hidden) render();
  });
  listen("setup-done", async () => {
    state.boot.setup_ready = true;
    await load();
    toast("Setup complete. Nabra is starting.");
    render();
  });
}
