// Usage numbers computed from local dictation history (newest first).
import { dayKey } from "./core.js";

const DAY = 86_400_000;

export function totals(history) {
  const words = history.reduce((n, d) => n + d.words, 0);
  const seconds = history.reduce((n, d) => n + d.seconds, 0);
  const fixes = history.reduce((f, d) => ({
    corrected: f.corrected + d.fixes.terms + d.fixes.ai,
    dictionary: f.dictionary + d.fixes.dictionary,
  }), { corrected: 0, dictionary: 0 });
  return { words, wpm: seconds > 5 ? (words / seconds) * 60 : 0, fixes };
}

/** Words per calendar day: Map(dayStartMs → words). */
export function perDay(history) {
  const m = new Map();
  for (const d of history) m.set(dayKey(d.id), (m.get(dayKey(d.id)) ?? 0) + d.words);
  return m;
}

/** Current streak counts back from today (or yesterday, if today has no dictation yet). */
export function streaks(history) {
  const days = perDay(history);
  let cur = 0;
  let day = dayKey(Date.now());
  if (!days.has(day)) day -= DAY;
  while (days.has(day)) { cur++; day -= DAY; }
  let longest = 0, run = 0, prev = null;
  for (const d of [...days.keys()].sort((a, b) => a - b)) {
    run = prev !== null && Math.round((d - prev) / DAY) === 1 ? run + 1 : 1;
    longest = Math.max(longest, run);
    prev = d;
  }
  return { current: cur, longest };
}

/** Words by app, biggest first: [{ app, words, share }]. */
export function byApp(history) {
  const m = new Map();
  for (const d of history) m.set(d.app || "Other", (m.get(d.app || "Other") ?? 0) + d.words);
  const total = [...m.values()].reduce((a, b) => a + b, 0) || 1;
  return [...m.entries()].map(([app, words]) => ({ app, words, share: words / total })).sort((a, b) => b.words - a.words);
}

const FRIENDLY = { Code: "VS Code", chrome: "Chrome", msedge: "Edge", firefox: "Firefox", WINWORD: "Word", OUTLOOK: "Outlook",
  Notepad: "Notepad", notepad: "Notepad", Discord: "Discord", slack: "Slack", WhatsApp: "WhatsApp", Telegram: "Telegram",
  "ms-teams": "Teams", Teams: "Teams", WindowsTerminal: "Terminal", ChatGPT: "ChatGPT", Claude: "Claude", EXCEL: "Excel",
  POWERPNT: "PowerPoint", Obsidian: "Obsidian", explorer: "File Explorer" };
export const appName = (exe) => FRIENDLY[exe] ?? (exe || "Other");
