// Insights: speed, fixes, totals, app usage and the streak calendar, all from local history.
import { $, h, fmt, state, dayKey, languageName } from "./core.js";

let tab = "usage";
import { totals, streaks, perDay, byApp, appName } from "./stats.js";

const WEEKS = 18;
const DAY = 86_400_000;

export function render() {
  const tabs = h("div", { class: "tabs" }, [["usage", "Your usage"], ["voice", "Your voice"]].map(([id, name]) =>
    h("button", { "aria-selected": String(tab === id), onclick: () => { tab = id; render(); } }, name)));
  if (tab === "voice") return voice(tabs);
  const t = totals(state.history);
  const s = streaks(state.history);
  const apps = byApp(state.history);
  $("#page-insights").replaceChildren(
    h("div", { class: "page-head" }, h("h1", {}, "Insights")),
    tabs,
    h("div", { class: "grid-3" },
      h("div", { class: "card metric" }, h("div", { class: "big" }, fmt(t.wpm)), h("div", { class: "label" }, "Words per minute"), gauge(t.wpm)),
      h("div", { class: "card metric" },
        h("div", { class: "big" }, fmt(t.fixes.corrected + t.fixes.dictionary)), h("div", { class: "label" }, "Fixes made by Nabra"), h("hr"),
        h("div", { class: "kv" }, h("span", {}, h("b", {}, fmt(t.fixes.corrected)), "words corrected")),
        h("div", { class: "kv" }, h("span", {}, h("b", {}, fmt(t.fixes.dictionary)), "dictionary fixes"))),
      h("div", { class: "card metric" },
        h("div", { class: "big" }, fmt(t.words)), h("div", { class: "label" }, "Total words dictated"), h("hr"),
        h("div", { class: "kv" }, h("span", {}, "This PC"), h("span", {}, `${fmt(t.words)} words`)),
        h("div", { class: "kv" }, h("span", {}, "Dictations"), h("span", {}, fmt(state.history.length))))),
    h("div", { class: "grid-2" },
      h("div", { class: "card metric" },
        h("div", { class: "head-row" }, h("h3", {}, "App usage"), h("span", { class: "label" }, `Total apps used | ${apps.length}`)),
        apps.length ? apps.slice(0, 6).map((a, i) => h("div", { class: "usage-row" },
          h("div", { class: "usage-bar", style: `width:${Math.max(8, a.share * 100)}%;opacity:${1 - i * 0.12}` }, `${Math.round(a.share * 100)}%`),
          h("span", { class: "label" }, `${fmt(a.words)} · ${appName(a.app)}`)))
          : h("p", { class: "muted" }, "Dictate in a few apps to see where you use your voice most.")),
      h("div", { class: "card metric" },
        h("div", { class: "head-row" }, h("h3", {}, `${s.current} day streak`), h("span", { class: "label" }, `Longest streak | ${s.longest} days`)),
        heatmap(),
        h("div", { class: "heat-legend" }, "Less", ["l1", "l2", "l3", "l4"].map((l) => h("i", { class: l })), "More"))),
  );
}

/** Semicircle gauge; 200 wpm = full. */
function gauge(wpm) {
  const frac = Math.min(1, wpm / 200);
  const r = 62, cx = 80, cy = 78, len = Math.PI * r;
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("viewBox", "0 0 160 92");
  svg.setAttribute("class", "gauge");
  svg.setAttribute("role", "img");
  svg.setAttribute("aria-label", `${Math.round(wpm)} words per minute`);
  const arc = (stroke, dash) => {
    const p = document.createElementNS(ns, "path");
    p.setAttribute("d", `M ${cx - r} ${cy} A ${r} ${r} 0 0 1 ${cx + r} ${cy}`);
    p.setAttribute("stroke", stroke);
    p.setAttribute("stroke-width", "16");
    p.setAttribute("stroke-linecap", "round");
    p.setAttribute("fill", "none");
    if (dash != null) p.setAttribute("stroke-dasharray", `${dash} ${len}`);
    return p;
  };
  const label = (y, text, cls) => {
    const t = document.createElementNS(ns, "text");
    t.setAttribute("x", cx); t.setAttribute("y", y); t.setAttribute("text-anchor", "middle");
    if (cls) t.setAttribute("class", cls);
    t.textContent = text;
    return t;
  };
  const speed = wpm >= 150 ? "Very fast" : wpm >= 110 ? "Fast" : wpm >= 70 ? "Steady" : wpm > 0 ? "Relaxed" : "No data";
  svg.append(arc("var(--sunk)"), arc("var(--accent)", frac * len), label(66, speed), label(82, "vs typing ≈40 wpm", "small"));
  return svg;
}

/** GitHub-style calendar: WEEKS columns × 7 rows (Sun..Sat), colored by words that day. */
function heatmap() {
  const words = perDay(state.history);
  const today = dayKey(Date.now());
  const end = today + (6 - new Date(today).getDay()) * DAY; // Saturday of this week
  const start = end - (WEEKS * 7 - 1) * DAY;
  const max = Math.max(1, ...words.values());
  const grid = h("div", { class: "heat", style: `--weeks:${WEEKS}` });
  ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"].forEach((name, dow) => {
    grid.append(h("span", {}, name));
    for (let w = 0; w < WEEKS; w++) {
      const day = start + (w * 7 + dow) * DAY;
      const n = words.get(day) ?? 0;
      const level = day > today || !n ? "" : `l${Math.min(4, 1 + Math.floor((n / max) * 3.999))}`;
      grid.append(h("i", { class: `${level}${day === today ? " today" : ""}`,
        title: `${new Date(day).toLocaleDateString()}: ${n} words` }));
    }
  });
  return grid;
}

// ---------- your voice ----------
const COLORS = ["var(--accent)", "var(--accent-2)", "#d68c40", "#7a5cc7", "#c2577a", "var(--soft)"];
const STOP = new Set("that this with have from your they what when then there their will would just like about into been were also than them some more very okay yeah".split(" "));

function voice(tabs) {
  const h_ = state.history;
  const langs = new Map();
  for (const d of h_) langs.set(d.language || "?", (langs.get(d.language || "?") ?? 0) + d.words);
  const total = [...langs.values()].reduce((a, b) => a + b, 0) || 1;
  const mix = [...langs.entries()].sort((a, b) => b[1] - a[1]);

  const hours = new Array(24).fill(0);
  for (const d of h_) hours[new Date(d.id).getHours()] += d.words;
  const peak = Math.max(1, ...hours);

  const counts = new Map();
  for (const d of h_) {
    for (const raw of d.text.toLowerCase().split(/\s+/)) {
      const w = raw.replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}.]+$/gu, "");
      const arabic = /[\u0600-\u06FF]/.test(w);
      if ((arabic ? w.length >= 3 : w.length >= 4) && !STOP.has(w)) counts.set(w, (counts.get(w) ?? 0) + 1);
    }
  }
  const top = [...counts.entries()].sort((a, b) => b[1] - a[1]).slice(0, 24);
  const avg = h_.length ? Math.round(h_.reduce((n, d) => n + d.words, 0) / h_.length) : 0;
  const longest = h_.reduce((m, d) => Math.max(m, d.words), 0);

  if (!h_.length) {
    return $("#page-insights").replaceChildren(h("div", { class: "page-head" }, h("h1", {}, "Insights")), tabs,
      h("p", { class: "empty" }, "Dictate a few times and your voice profile appears here."));
  }
  $("#page-insights").replaceChildren(
    h("div", { class: "page-head" }, h("h1", {}, "Insights")), tabs,
    h("div", { class: "grid-3" },
      h("div", { class: "card metric" }, h("div", { class: "big" }, fmt(avg)), h("div", { class: "label" }, "Words per dictation")),
      h("div", { class: "card metric" }, h("div", { class: "big" }, fmt(longest)), h("div", { class: "label" }, "Longest dictation (words)")),
      h("div", { class: "card metric" }, h("div", { class: "big" }, String(mix.filter(([c]) => c !== "?").length)), h("div", { class: "label" }, "Languages you dictate in"))),
    h("div", { class: "card metric", style: "margin-bottom:18px" },
      h("div", { class: "head-row" }, h("h3", {}, "Language mix"), h("span", { class: "label" }, "By words")),
      h("div", { class: "mix" }, mix.map(([code, n], i) => h("span", { style: `width:${(n / total) * 100}%;background:${COLORS[Math.min(i, COLORS.length - 1)]}` },
        n / total > 0.08 ? `${Math.round((n / total) * 100)}%` : ""))),
      h("div", { class: "legend-row" }, mix.map(([code, n], i) => h("span", {}, h("i", { style: `background:${COLORS[Math.min(i, COLORS.length - 1)]}` }),
        `${code === "?" ? "Unknown" : languageName(code)} · ${fmt(n)} words`))),
      h("p", { class: "muted", style: "margin:12px 0 0;font-size:13px" }, "Mixed sentences count toward the language they mostly use.")),
    h("div", { class: "grid-2" },
      h("div", { class: "card metric" }, h("div", { class: "head-row" }, h("h3", {}, "When you dictate"), h("span", { class: "label" }, "Words by hour")),
        h("div", { class: "hours" }, hours.map((n, hr) => h("i", { style: `height:${(n / peak) * 100}%`, title: `${hr}:00 · ${n} words` }))),
        h("div", { class: "hours-axis" }, ["12am", "6am", "12pm", "6pm", "11pm"].map((t) => h("span", {}, t)))),
      h("div", { class: "card metric" }, h("div", { class: "head-row" }, h("h3", {}, "Words you use most")),
        h("div", { class: "words-cloud" }, top.map(([w, n]) => h("span", { dir: "auto" }, w, h("small", {}, String(n))))))),
  );
}
