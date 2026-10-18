// What the window shows before a shell loads: where the status page file is,
// and the plain screen drawn in its place when that file cannot be loaded
// (`DesktopHost.render`).

import path from "node:path";
import type { FailureReason } from "./cli";

const ARCHIVE = /([\\/])app\.asar(?=[\\/])/;

/**
 * The status page's file. With the shipped `GrantFileProtocolExtraPrivileges`
 * fuse off (`scripts/fuses.mjs`) Electron's `file:` loader no longer reads
 * inside an asar archive: a `loadFile` of a path under `app.asar` answers
 * ERR_FILE_NOT_FOUND for a file the archive lists. The packager unpacks
 * `dist/static` (`scripts/package.mjs`), so the page is a plain file beside
 * the archive; an unpackaged run has no archive in its path and keeps it.
 */
export function statusPageFile(dirname: string): string {
  return path.join(dirname, "static", "status.html").replace(ARCHIVE, "$1app.asar.unpacked");
}

/** The reasons that name their own title (the app or the core is the one to update); `static/status.js` keeps the same list. */
const TITLES: Partial<Record<FailureReason, string>> = { core_newer: "core_newer_title", update_failed: "update_failed_title" };
const REASONS = new Set<string>(["cli_missing", "start_failed", "no_response", "other_build", "state_refused", "core_newer", "update_failed"]);

export type FallbackScreen = {
  lang: string;
  heading: string | null;
  reason: string;
  reasonCode: string | null;
  lines: string[];
  retry: string;
};

/**
 * What `static/status.js` would draw for `failed`, from the same sentences
 * (`words`, the status page's query): the heading, the reason sentence, and
 * the lines under it. `said` replaces the reason sentence when the host
 * wrote one for this failure.
 */
export function fallbackScreen(
  words: Record<string, string>,
  state: { kind: "connecting" } | { kind: "failed"; reason: string; file?: string; said?: string; builds?: string },
): FallbackScreen {
  // A missing sentence is a host bug and fails loudly, as the page does.
  const word = (key: string): string => {
    const value = words[key];
    if (value === undefined) throw new Error(`status screen has no ${key} text`);
    return value;
  };
  const base = { lang: word("lang"), retry: word("retry") };
  if (state.kind === "connecting") return { ...base, heading: null, reason: word("connecting"), reasonCode: null, lines: [] };
  const known = REASONS.has(state.reason) ? (state.reason as FailureReason) : "start_failed";
  const title = TITLES[known];
  return {
    ...base,
    heading: word(title ?? "failed"),
    reason: state.said ?? word(title ? "start_failed" : known),
    reasonCode: state.reason,
    lines: [state.file, state.builds].filter((line): line is string => line !== undefined && line !== ""),
  };
}

/**
 * The script that draws `screen` into a blank page. The page is `about:blank`
 * because that is a document the window can load with nothing to read, and
 * whose Retry link, a fragment of its own address, reaches the host as an
 * in-page navigation like the status page's (a `data:` page does not take
 * one). Every sentence is set as text, never as markup. Colors are the
 * system's for a dark page, since the status page's tokens are in the file
 * that did not load; the window's own background is the host's token.
 */
export function fallbackScript(screen: FallbackScreen, background: string): string {
  return `(() => {
  const screen = ${JSON.stringify(screen)};
  const root = document.documentElement;
  root.lang = screen.lang;
  root.style.cssText = "color-scheme:dark;background:" + ${JSON.stringify(background)};
  document.title = "hide";
  const body = document.body;
  body.style.cssText = "margin:0;height:100vh;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:8px;text-align:center;font:13px system-ui,sans-serif;color:CanvasText;-webkit-user-select:none";
  const add = (tag, text, css) => {
    const element = document.createElement(tag);
    element.textContent = text;
    element.style.cssText = css;
    body.append(element);
    return element;
  };
  body.replaceChildren();
  if (screen.heading !== null) add("h1", screen.heading, "margin:0;font-size:16px;font-weight:600");
  const reason = add("p", screen.reason, "margin:0;max-width:420px;color:" + (screen.heading === null ? "GrayText" : "CanvasText"));
  if (screen.reasonCode !== null) {
    reason.id = "reason";
    reason.dataset.reason = screen.reasonCode;
  }
  for (const line of screen.lines) add("p", line, "margin:0;font:12px ui-monospace,Menlo,monospace;color:GrayText;overflow-wrap:anywhere");
  if (screen.reasonCode !== null) {
    const retry = add("a", screen.retry, "margin-top:8px;padding:6px 14px;border-radius:6px;background:ButtonFace;color:ButtonText;text-decoration:none");
    retry.href = "#retry";
    retry.setAttribute("role", "button");
  }
})()`;
}
