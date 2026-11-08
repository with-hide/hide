// The first flow's world: the introduction site's hero window (a lead, the agent
// asking to merge, and the Project tool beside them), which shows all three
// parts of the screen with work in them. Its snapshot is `../data/tour.json`,
// kept as the core would send it with each time written as how long before
// now it was. It answers the sidebar's list, so the learner can turn it
// through its three lenses, and notes that ⌘K was opened.

import { nextSidebarLens, type SidebarLens } from "../../snapshot";
import tour from "../data/tour.json";
import type { PracticeWorld, WorldSnapshot } from "../world";

/** `searched` is whether ⌘K was opened, which asks the core for sessions; `tool` is the tool the lane shows, none while it is closed. */
export type TourModel = { lens: SidebarLens; seen: SidebarLens[]; searched: boolean; tool: "project" | null };

/** A time in the data file: milliseconds, seconds or an ISO time that long before now, or a date that many days before today. */
type Ago = { $msAgo?: number; $sAgo?: number; $isoAgo?: number; $daysAgo?: number };

const DAY_MS = 86_400_000;

/** The data file's values with each time placed that long before `nowMs`. */
function dated(value: unknown, nowMs: number): unknown {
  if (Array.isArray(value)) return value.map((item) => dated(item, nowMs));
  if (!value || typeof value !== "object") return value;
  const ago = value as Ago;
  if (ago.$msAgo !== undefined) return nowMs - ago.$msAgo;
  if (ago.$sAgo !== undefined) return nowMs / 1000 - ago.$sAgo;
  if (ago.$isoAgo !== undefined) return new Date(nowMs - ago.$isoAgo).toISOString();
  if (ago.$daysAgo !== undefined) return new Date(nowMs - ago.$daysAgo * DAY_MS).toISOString().slice(0, 10);
  return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, dated(item, nowMs)]));
}

const CLAUDE = "\x1b[38;2;215;119;87m";
const CODEX = "\x1b[36m";
const DIM = "\x1b[90m";
const BOLD = "\x1b[1m";
const GREEN = "\x1b[32m";
const RESET = "\x1b[0m";
const lines = (...rows: string[]) => rows.join("\r\n");
/** A pull request reference as the terminal writes it. */
const ref = (number: number) => `#${number}`;
/** Where a terminal's full-width rule goes; it is drawn as wide as the terminal. */
const RULE = "{rule}";

/** The two panes side by side, and what each terminal shows. */
const TERMINALS: Record<string, string> = {
  a1: lines(
    "",
    "",
    `${CLAUDE}✻${RESET} ${BOLD}Claude Code${RESET}`,
    `  ${DIM}~/billing-app · main${RESET}`,
    "",
    `${DIM}>${RESET} Split the invoice export in three`,
    "",
    "● Three parts, one Codex worker each.",
    "",
    `● ${BOLD}Bash${RESET}(hide agent spawn …)`,
    `  ${DIM}⎿${RESET}  Export invoices as CSV`,
    `● ${BOLD}Bash${RESET}(hide agent spawn …)`,
    `  ${DIM}⎿${RESET}  Sort the export by date`,
    `● ${BOLD}Bash${RESET}(hide agent spawn …)`,
    `  ${DIM}⎿${RESET}  Add the export button`,
    "",
    "● The workers report back to me, and I",
    "  check each pull request before you.",
    "",
    `${CLAUDE}✻ Waiting on 3 workers… ${DIM}(6m 12s)${RESET}`,
    "",
    `${DIM}${RULE}${RESET}`,
    `${DIM}>${RESET} `,
    `${DIM}${RULE}${RESET}`,
    `  ${DIM}⏵⏵ accept edits on${RESET}`,
  ),
  a3: lines(
    "",
    `${DIM}╭──────────────────────────────╮${RESET}`,
    `${DIM}│${RESET} ${BOLD}>_ Codex${RESET}                     ${DIM}│${RESET}`,
    `${DIM}│${RESET} ${DIM}~/billing-app · main${RESET}         ${DIM}│${RESET}`,
    `${DIM}╰──────────────────────────────╯${RESET}`,
    "",
    `${CODEX}›${RESET} Check ${ref(431)} and ${ref(436)}, then ask me`,
    "",
    `${DIM}•${RESET} ${BOLD}Ran${RESET} pnpm test export`,
    `  ${DIM}└${RESET} ${GREEN}38 passed${RESET}`,
    "",
    `${DIM}•${RESET} Both pull requests are ready:`,
    `  ${ref(431)} Export invoices as CSV`,
    `  ${ref(436)} Add the export button`,
    "",
    `${DIM}${RULE}${RESET}`,
    ` ${BOLD}Merge both into main?${RESET}`,
    "",
    ` ${CODEX}› 1. Yes, merge both${RESET}`,
    `   2. Only ${ref(431)}`,
    "   3. Not yet",
  ),
};

export const TOUR_WORLD: PracticeWorld<TourModel> = {
  initial: () => ({ lens: "projects", seen: ["projects"], searched: false, tool: null }),
  apply(model, event) {
    if (event.kind === "session_work_search") return model.searched ? null : { ...model, searched: true };
    // The rail's tool pressed opens the lane on it, and pressed again, or closed, puts it away.
    if (event.kind === "workspace_view") {
      const tool = event.payload.open === false ? null : "tool" in event.payload ? (event.payload.tool === "project" ? "project" : null) : model.tool;
      return tool === model.tool ? null : { ...model, tool };
    }
    if (event.kind !== "sidebar_lens_set") return null;
    const lens = (event.payload.lens as SidebarLens | undefined) ?? nextSidebarLens(model.lens);
    return { ...model, lens, seen: model.seen.includes(lens) ? model.seen : [...model.seen, lens] };
  },
  snapshot(model, nowMs) {
    const { rest, agents, usage } = dated(tour, nowMs) as Omit<WorldSnapshot, "focused">;
    // The window opens with the lane closed: nothing has asked for a tool yet.
    const view = { ...rest.workspace_view!, open: model.tool !== null, tool: model.tool, pinned: false };
    return { rest: { ...rest, ui_state: { ...rest.ui_state, sidebar_lens: model.lens }, workspace_view: view }, agents, focused: null, usage };
  },
  panes: () => Object.keys(TERMINALS),
  screen: (_model, paneId, cols) => TERMINALS[paneId]!.replaceAll(RULE, "─".repeat(cols)),
};
