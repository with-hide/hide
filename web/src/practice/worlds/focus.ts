// Focus mode's world: one invented project with two agents side by side in a
// tab, one of them building a small page and having opened it as a View on
// its desk. It answers focus mode as the core does (`pair_view`,
// `pair_view_desk`, and a pane chosen while the mode is on following it): the
// pane on the left and, for the agent that has a View, that View beside it,
// or the pane alone for the other. A few seconds after the mode is first
// entered the agent changes the page's heading (`later`), and the page beside
// the terminal changes with it.
//
// The page is a Markdown file the Editor draws: a browser page is a native
// view the desktop host lays over the screen, which a practice screen has
// none of, and a heading is as easy to see change.

import type { AgentRow, EditorDocumentSnapshot, EditorTabSnapshot, PaneHeader, ViewDisplaySnapshot } from "../../snapshot";
import type { CoreEvent, PracticeWorld, WorldSnapshot } from "../world";
import { agent, applyDesk, deskSnapshot, rowRequest, rowState, shellScreen, type DeskModel, type PracticePane, type PracticePlace } from "./kit";

const PLACE: PracticePlace = { project: "hide-practice", label: "hide-practice", branch: "main" };
const ROOT = `/work/${PLACE.project}`;
const MINUTE = 60_000;

/** The two agents' panes: the builder, whose page is on its desk, and one with no View. */
export const BUILDER = "lead";
export const OTHER = "docs";

const TITLE = { [BUILDER]: "Build the sale page", [OTHER]: "Update the README" } as const;
const PAGE_FILE = "sale.md";
const PAGE_PATH = `${ROOT}/${PAGE_FILE}`;
const PAGE_VIEW = "view1";
const EDITOR_TAB = "file1";
const TAB = "t1";

/** How long after the mode is first entered the agent changes the page, in ms. */
export const CHANGE_AFTER = 3000;
const BEAT = "practice_beat";

/** The page before and after the agent's edit: the heading is what the eye finds. */
const PAGES = [
  ["# Spring sale", "", "Everything in the shop is 20% off until Sunday.", "", "- Free shipping over $30", ""],
  ["# Summer sale", "", "Everything in the shop is 30% off until Sunday.", "", "- Free shipping over $30", "- Free returns", ""],
].map((lines) => lines.join("\n"));

/** The pane focus mode draws on the left, and whether the desk was folded by the learner (⌘E). */
type Pair = { pane: string; folded: boolean };

export type FocusModel = DeskModel & {
  /** Focus mode while it is on. */
  pair: Pair | null;
  /** The page's stage: 0 as the agent opened it, 1 once the agent changed its heading. */
  stage: 0 | 1;
  /** When the mode was first entered, which paces the agent's edit; null before. */
  enteredAt: number | null;
  /** Whether the agent's page is still open on its desk. */
  pageOpen: boolean;
  /** Whether the operator has looked at the page: the mode drawing it beside its pane is looking. */
  seen: boolean;
};

const hasPage = (model: FocusModel, pane: string) => model.pageOpen && pane === BUILDER;

/** Whether focus mode draws the desk beside its pane: the pane has a page and the desk is not folded. */
export const besidePage = (model: FocusModel) => model.pair !== null && hasPage(model, model.pair.pane) && !model.pair.folded;
/** Whether focus mode draws the builder with its page beside it. */
export const focusedOnBuilder = (model: FocusModel) => besidePage(model) && model.pair?.pane === BUILDER;
/** Whether the agent has changed the page. */
export const pageChanged = (model: FocusModel) => model.stage >= 1;
/** Whether focus mode is off. */
export const focusEnded = (model: FocusModel) => model.pair === null;

function initial(): FocusModel {
  const row = (pane: keyof typeof TITLE, kind: string, minutes: number) =>
    agent({ pane_id: pane, identity_label: TITLE[pane], agent_kind: kind, group: "working", mark: "going", status_code: "working", activity: "working", elapsed: `${minutes}m` });
  const panes: PracticePane[] = [
    { id: BUILDER, title: TITLE[BUILDER], agent: row(BUILDER, "claude", 4) },
    { id: OTHER, title: TITLE[OTHER], agent: row(OTHER, "codex", 7) },
  ];
  return {
    panes,
    tabs: [{ id: TAB, name: null, root: { type: "split", direction: "right", ratio: 0.5, first: { type: "pane", pane_id: BUILDER }, second: { type: "pane", pane_id: OTHER } }, focused: BUILDER, zoomed: false }],
    groups: [{ id: null, name: null, tabs: [TAB] }],
    selectedGroup: null, activeTab: TAB, nextId: 3, operations: [],
    pair: null, stage: 0, enteredAt: null, pageOpen: true, seen: false,
  };
}

// --- The core's answers ------------------------------------------------------------------------

/** `pair_view {pane_id?}`: the pane to draw on the left, which turns the mode on, or none, which turns it off everywhere. */
function setPair(model: FocusModel, p: Record<string, unknown>): FocusModel | null {
  if (typeof p.pane_id !== "string") return model.pair === null ? null : { ...model, pair: null };
  const id = p.pane_id;
  if (!model.panes.some((pane) => pane.id === id)) return null;
  // The same pane again changes nothing and publishes nothing.
  if (model.pair?.pane === id) return null;
  const desk = applyDesk(model, { kind: "focus_pane", payload: { pane_id: id } });
  const next: FocusModel = { ...model, ...(desk ?? {}), pair: { pane: id, folded: false }, enteredAt: model.enteredAt ?? Date.now() };
  return { ...next, seen: model.seen || besidePage(next) };
}

/** `pair_view_desk {shown}`: ⌘E in the mode folds the desk beside the pane, or stands it again when it has a tab to show. */
function setDesk(model: FocusModel, p: Record<string, unknown>): FocusModel | null {
  if (model.pair === null || typeof p.shown !== "boolean") return null;
  if (besidePage(model) === p.shown) return null;
  if (p.shown && !hasPage(model, model.pair.pane)) return null;
  return { ...model, pair: { ...model.pair, folded: !p.shown } };
}

function apply(model: FocusModel, event: CoreEvent): FocusModel | null {
  const p = event.payload;
  switch (event.kind) {
    case BEAT:
      return p.stage === 1 && model.stage === 0 ? { ...model, stage: 1 } : null;
    case "pair_view":
      return setPair(model, p);
    case "pair_view_desk":
      return setDesk(model, p);
    // A pane chosen while the mode is on takes the mode with it (`follow_pair_focus`).
    case "focus_pane": {
      const desk = applyDesk(model, event);
      if (!desk) return null;
      const next: FocusModel = { ...model, ...desk };
      const id = String(p.pane_id);
      if (next.pair === null || next.pair.pane === id) return next;
      const moved: FocusModel = { ...next, pair: { pane: id, folded: false } };
      return { ...moved, seen: moved.seen || besidePage(moved) };
    }
    // The page's tab closed from the desk: the pane stays, drawn alone.
    case "view_layout":
      return p.action === "close" && p.display_id === PAGE_VIEW && model.pageOpen ? { ...model, pageOpen: false } : null;
    default: {
      const desk = applyDesk(model, event);
      return desk ? { ...model, ...desk } : null;
    }
  }
}

// --- The snapshot ------------------------------------------------------------------------------

const request = (pane: keyof typeof TITLE, nowMs: number, minutes: number) => rowRequest(TITLE[pane], TITLE[pane], nowMs - minutes * MINUTE, null);

function rowOf(pane: keyof typeof TITLE, kind: string, minutes: number, nowMs: number): AgentRow {
  return agent({
    pane_id: pane, identity_label: TITLE[pane], agent_kind: kind, group: "working", mark: "going", status_code: "working", activity: "working", elapsed: `${minutes}m`,
    state: rowState("working", true, nowMs - minutes * MINUTE), request: request(pane, nowMs, minutes),
  });
}

function headers(model: FocusModel): Record<string, PaneHeader> {
  const working = (pane: keyof typeof TITLE, progress: string): PaneHeader => ({ working: true, notice: null, band: null, request: { text: TITLE[pane], images: 0 }, progress });
  return {
    [BUILDER]: working(BUILDER, model.stage >= 1 ? "Editing sale.md" : "Writing sale.md"),
    [OTHER]: working(OTHER, "Editing README.md"),
  };
}

/** The desk of the front tab: the page the builder opened, in front, while it is open. */
function view(model: FocusModel, base: NonNullable<WorldSnapshot["rest"]["workspace_view"]>): NonNullable<WorldSnapshot["rest"]["workspace_view"]> {
  const pair = model.pair;
  const opened = model.pageOpen;
  const display: ViewDisplaySnapshot = {
    id: PAGE_VIEW, tab_id: EDITOR_TAB, path: PAGE_PATH, label: PAGE_FILE, kind: "file", committed: null, preview: false, state: "open", reason: null,
    opener: { name: TITLE[BUILDER], provider: "claude", device_id: "local", pane_id: BUILDER, resumable: false },
    owner_pane: BUILDER,
    ...(model.seen ? {} : { unseen: true as const }),
  };
  const layout = opened
    ? { root: { area: { id: "v1", active: display.id, displays: [display] } }, active_area: "v1", display_count: 1, limits: { areas: 4, depth: 3, displays: 32 } }
    : base.layout;
  return {
    ...base,
    tray: {
      ...base.tray, tab_id: TAB, shown: false, count: opened ? 1 : 0,
      pair: pair && { tab_id: TAB, pane_id: pair.pane, alone: !besidePage(model), shown: hasPage(model, pair.pane) ? 1 : 0, folded: [] },
    },
    pair_mode: { on: pair !== null, ratio: null },
    panes_with_views: opened ? [BUILDER] : [],
    layout,
  };
}

/** The Editor's tab and document for the page, as the core sends them while the page is open. */
function page(model: FocusModel): { editor?: { tabs: EditorTabSnapshot[]; active_tab_id: string }; documents?: Record<string, EditorDocumentSnapshot> } {
  if (!model.pageOpen) return {};
  const tab: EditorTabSnapshot = { id: EDITOR_TAB, device_id: "local", workspace_id: PLACE.project, checkout_id: `${PLACE.project}:${PLACE.branch}`, path: PAGE_PATH, label: PAGE_FILE, kind: "file", diff_committed: null, markdown_live: true, wrap: false, dirty: false, preview: false };
  const document: EditorDocumentSnapshot = { path: PAGE_PATH, language: "markdown", document_kind: "markdown", contents_utf8: PAGES[model.stage]!, revision: `sha256:${model.stage}`, dirty: false, readonly_reason: null, conflict: null, save: null };
  return { editor: { tabs: [tab], active_tab_id: EDITOR_TAB }, documents: { [EDITOR_TAB]: document } };
}

function snapshot(model: FocusModel, nowMs: number): WorldSnapshot {
  const rows = { [BUILDER]: rowOf(BUILDER, "claude", 4, nowMs), [OTHER]: rowOf(OTHER, "codex", 7, nowMs) };
  const desk: DeskModel = { ...model, panes: model.panes.map((pane) => ({ ...pane, agent: rows[pane.id as keyof typeof rows] ?? pane.agent })) };
  const drawn = deskSnapshot([{ place: PLACE, model: desk }], PLACE.project, nowMs);
  return {
    ...drawn,
    rest: {
      ...drawn.rest,
      ui_state: { ...drawn.rest.ui_state!, sidebar_lens: "projects" },
      workspace_view: view(model, drawn.rest.workspace_view!),
      terminal: { headers: headers(model) },
    },
    frames: page(model),
  };
}

// --- What the terminals show -------------------------------------------------------------------

const CLAUDE = "\x1b[38;2;215;119;87m";
const CODEX = "\x1b[36m";
const DIM = "\x1b[90m";
const BOLD = "\x1b[1m";
const GREEN = "\x1b[32m";
const RESET = "\x1b[0m";

function builderScreen(model: FocusModel): string {
  const rows = [
    "", "", `${CLAUDE}✻${RESET} ${BOLD}Claude Code${RESET}`, `  ${DIM}~/${PLACE.project} · ${PLACE.branch}${RESET}`,
    "", `${DIM}>${RESET} Make a page for the spring sale`, "",
    `${CLAUDE}●${RESET} ${BOLD}Write${RESET}(${PAGE_FILE})`, `  ${DIM}⎿${RESET}  ${GREEN}+ 5 lines${RESET}`,
    `${CLAUDE}●${RESET} ${BOLD}Bash${RESET}(hide file open ${PAGE_FILE})`, `  ${DIM}⎿${RESET}  Opened for you to look at`,
  ];
  if (model.stage >= 1) rows.push("", `${CLAUDE}●${RESET} It is summer now. Changing the heading.`, `${CLAUDE}●${RESET} ${BOLD}Update${RESET}(${PAGE_FILE})`, `  ${DIM}⎿${RESET}  ${GREEN}+ Summer sale${RESET}`);
  rows.push("", `${CLAUDE}✻ ${DIM}Working… (esc to interrupt)${RESET}`);
  return rows.join("\r\n");
}

function otherScreen(): string {
  return [
    "", "", `${BOLD}>_ Codex${RESET}`, "", `${CODEX}›${RESET} ${TITLE[OTHER]}`, "",
    `${DIM}•${RESET} ${BOLD}Edited${RESET} README.md`, `  ${DIM}└${RESET} writing the install steps…`,
  ].join("\r\n");
}

export const FOCUS_WORLD: PracticeWorld<FocusModel> = {
  initial,
  apply,
  snapshot,
  // The agent edits the page a little after the learner first looks at it beside the terminal, as an agent at work would.
  later: (model) => (model.enteredAt !== null && model.stage === 0 ? { at: model.enteredAt + CHANGE_AFTER, event: { kind: BEAT, payload: { stage: 1 } } } : null),
  panes: (model) => model.panes.map((pane) => pane.id),
  screen: (model, paneId) => (paneId === BUILDER ? builderScreen(model) : paneId === OTHER ? otherScreen() : shellScreen(PLACE)),
};
