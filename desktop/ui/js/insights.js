// Insights: speed, fixes, totals, app usage and the streak calendar, all from local history.
import { $, h, fmt, state, dayKey } from "./core.js";
import { totals, streaks, perDay, byApp, appName } from "./stats.js";

const WEEKS = 18;
const DAY = 86_400_000;

export function render() {
  const t = totals(state.history);
  const s = streaks(state.history);
  const apps = byApp(state.history);
  $("#page-insights").replaceChildren(
    h("div", { class: "page-head" }, h("h1", {}, "Insights")),
    h("div", { class: "tabs" }, h("button", { "aria-selected": "true" }, "Your usage")),
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
