// Desk's room in this window (PRD desk-2-ui D-01, D-05, D-08, D-12-D-15,
// D-22, B1-B26): which session is open beside Desk, and the tab panel that
// floats at that session's left edge, or at the window's right edge when
// none is open (B38, B39). All of it is this window's and starts closed every
// time the room is entered (B26); the core keeps only the two panels' widths
// (`desk.lane`, B23).

import type { DeskOutput } from "./snapshot";
import { absolutePathRoot } from "./terminalLinks";

/**
 * A tab of the tab panel (B18, B19): an output the session opened (a page,
 * an image, a video, a document, or a PR, drawn by Desk's output viewer), a
 * file of a checkout (the app's file view), or a mission's map.
 */
export type DeskTab =
  | { kind: "output"; output: DeskOutput }
  | { kind: "file"; device_id: string; root: string; path: string }
  | { kind: "mission"; mission: string; title: string };

/** The panel's tabs, the italic preview among them, the tab in front, and the Explorer's tool column (B18, B35, B36). */
export type DeskPanel = {
  tabs: DeskTab[];
  /** The key of the italic tab the next single press replaces (B36). */
  preview: string | null;
  /** The key of the tab in front. */
  active: string | null;
  /** The Explorer's tool column at the panel's left (B35). */
  explorer: boolean;
  /** The key of the file tab a second file view shows beside the one in front, the same file or another (⌘↵, D-23, B37); the window's, so no Workspace's layout holds it. */
  beside: string | null;
};

export type DeskRoomWindow = {
  /** The pane whose session is open beside Desk, null while the right side is closed (B1, B2). */
  session: string | null;
  /** The tab panel, null while it is closed (B16, B21). */
  panel: DeskPanel | null;
  /** Moves each time `답하기` or `직접 쓰기` asks for the session's input box, so it takes the keyboard once per press. */
  answer: number;
};

export const DESK_ROOM_CLOSED: DeskRoomWindow = { session: null, panel: null, answer: 0 };

/** Where the tab panel stands: attached to the session panel's left edge, or at the window's right edge with no session open (D-23, D-24, B38, B39). */
export type DeskPanelPlace = "beside_session" | "window_edge";

export function panelPlace(room: DeskRoomWindow): DeskPanelPlace {
  return room.session !== null ? "beside_session" : "window_edge";
}

/** A tab's identity: one tab per output, per file of a checkout, per mission. */
export function tabKey(tab: DeskTab): string {
  switch (tab.kind) {
    case "output":
      return `output:${tab.output.id}`;
    case "file":
      return `file:${tab.device_id}:${tab.root}:${tab.path}`;
    case "mission":
      return `mission:${tab.mission}`;
  }
}

/**
 * A session opened beside Desk (B2, B3, B13, B20, B47). Another session's
 * tabs are not this one's, so an open tab panel closes (D-14, B25); the same
 * session again keeps its panel.
 */
export function openSession(room: DeskRoomWindow, pane: string, options: { answer?: boolean } = {}): DeskRoomWindow {
  const answer = options.answer ? room.answer + 1 : room.answer;
  if (room.session === pane) return { ...room, answer };
  return { session: pane, panel: null, answer };
}

/** The session panel's `✕` or Escape (B6): Desk widens again and the tab panel, which was that session's, goes with it. */
export function closeSession(room: DeskRoomWindow): DeskRoomWindow {
  return { ...room, session: null, panel: null };
}

/**
 * The open contract (D-05, D-08, D-23, D-24, B16, B36-B39): a file, an
 * output, a PR or a mission opens as a tab of the tab panel, opening the
 * panel when it is closed; where the panel stands is `panelPlace`. A tab
 * already there comes to the front. A preview open (a single press in the
 * Explorer) replaces the italic preview tab; any other open is kept, and
 * keeping the preview stops calling it the preview.
 */
export function openTab(room: DeskRoomWindow, tab: DeskTab, options: { preview?: boolean; beside?: boolean } = {}): DeskRoomWindow {
  const panel = room.panel ?? EMPTY_PANEL;
  const key = tabKey(tab);
  if (options.beside && tab.kind === "file") return openBeside(room, panel, tab, key);
  const present = panel.tabs.some((one) => tabKey(one) === key);
  if (present) {
    const preview = panel.preview === key && !options.preview ? null : panel.preview;
    return { ...room, panel: { ...panel, active: key, preview } };
  }
  if (options.preview && panel.preview !== null) {
    // The preview is replaced in its own place.
    const tabs = panel.tabs.map((one) => (tabKey(one) === panel.preview ? tab : one));
    return { ...room, panel: { ...panel, tabs, preview: key, active: key } };
  }
  return { ...room, panel: { ...panel, tabs: [...panel.tabs, tab], preview: options.preview ? key : panel.preview, active: key } };
}

const EMPTY_PANEL: DeskPanel = { tabs: [], preview: null, active: null, explorer: false, beside: null };

/**
 * ⌘↵ (D-23, B37): the file opens in a second file view beside the one in
 * front, a kept tab; the same file is shown twice. With nothing in front it
 * is an ordinary open.
 */
function openBeside(room: DeskRoomWindow, panel: DeskPanel, tab: DeskTab, key: string): DeskRoomWindow {
  const front = activeTab(panel);
  if (!front) return openTab(room, tab);
  const present = panel.tabs.some((one) => tabKey(one) === key);
  const tabs = present ? panel.tabs : [...panel.tabs, tab];
  const preview = panel.preview === key ? null : panel.preview;
  return { ...room, panel: { ...panel, tabs, preview, active: tabKey(front), beside: key } };
}

/** A side's `✕` (B37): the view beside goes, or the one in front goes and the one beside takes its place; the tab stays. */
export function closeSide(room: DeskRoomWindow, side: "front" | "beside"): DeskRoomWindow {
  const panel = room.panel;
  if (!panel?.beside) return room;
  if (side === "beside") return { ...room, panel: { ...panel, beside: null } };
  return { ...room, panel: { ...panel, active: panel.beside, beside: null } };
}

/** A double press on the preview tab keeps it (B36). */
export function keepTab(room: DeskRoomWindow, key: string): DeskRoomWindow {
  if (!room.panel || room.panel.preview !== key) return room;
  return { ...room, panel: { ...room.panel, preview: null } };
}

export function showTab(room: DeskRoomWindow, key: string): DeskRoomWindow {
  if (!room.panel?.tabs.some((one) => tabKey(one) === key)) return room;
  return { ...room, panel: { ...room.panel, active: key } };
}

/** A tab's `✕`: the tab beside it comes to the front; the panel closes with its last tab unless the Explorer column holds it open. */
export function closeTab(room: DeskRoomWindow, key: string): DeskRoomWindow {
  const panel = room.panel;
  if (!panel) return room;
  const index = panel.tabs.findIndex((one) => tabKey(one) === key);
  if (index < 0) return room;
  const tabs = panel.tabs.filter((_, at) => at !== index);
  if (tabs.length === 0 && !panel.explorer) return { ...room, panel: null };
  const next = tabs[Math.min(index, tabs.length - 1)];
  const active = panel.active === key ? (panel.beside && panel.beside !== key ? panel.beside : next ? tabKey(next) : null) : panel.active;
  const beside = panel.beside === key || panel.beside === active ? null : panel.beside;
  return { ...room, panel: { ...panel, tabs, active, beside, preview: panel.preview === key ? null : panel.preview } };
}

/** `✕`, Escape or a press on the dimmed outside (B21): the panel closes and the session stays open. */
export function closePanel(room: DeskRoomWindow): DeskRoomWindow {
  return { ...room, panel: null };
}

/**
 * `파일 ⇧⌘E` (D-22, B35): the panel with the Explorer's tool column, or that
 * column closed when it shows; a panel left with no tab closes. Only a
 * session's checkout has an Explorer, so with no session it does nothing.
 */
export function toggleExplorer(room: DeskRoomWindow): DeskRoomWindow {
  if (room.session === null) return room;
  const panel = room.panel;
  if (!panel) return { ...room, panel: { ...EMPTY_PANEL, explorer: true } };
  if (!panel.explorer) return { ...room, panel: { ...panel, explorer: true } };
  return panel.tabs.length === 0 ? { ...room, panel: null } : { ...room, panel: { ...panel, explorer: false } };
}

/** The tab in front: the one chosen, else the last. */
export function activeTab(panel: DeskPanel | null): DeskTab | null {
  if (!panel) return null;
  return panel.tabs.find((one) => tabKey(one) === panel.active) ?? panel.tabs.at(-1) ?? null;
}

/** A file's name, the last part of its path. */
export function fileName(path: string): string {
  const parts = path.split("/").filter(Boolean);
  return parts.at(-1) ?? path;
}

/**
 * A tab's name as its tab draws it (D-25, B40): a file's name, followed by
 * its project only while another file tab has the same name; any other tab
 * its own label.
 */
export function tabNames(tabs: readonly DeskTab[], projectOf: (tab: Extract<DeskTab, { kind: "file" }>) => string): Map<string, { name: string; project: string | null }> {
  const counts = new Map<string, number>();
  for (const tab of tabs) if (tab.kind === "file") counts.set(fileName(tab.path), (counts.get(fileName(tab.path)) ?? 0) + 1);
  return new Map(tabs.map((tab) => {
    if (tab.kind !== "file") return [tabKey(tab), { name: tab.kind === "output" ? tab.output.label : tab.title, project: null }];
    const name = fileName(tab.path);
    return [tabKey(tab), { name, project: (counts.get(name) ?? 0) > 1 ? projectOf(tab) : null }];
  }));
}

/**
 * Whether `written`, a file as a session's call typed it, names the View
 * `made` opened (B16). A View's path is spelled as the wire spells it, `/`
 * between names on every system, so a Windows device's backslashes are read
 * as `/` and its drive letter in either case (docs/ARCHITECTURE.md, The
 * platform layer); a relative path names the View whose path ends with it.
 */
export function namesView(made: DeskOutput, written: string): boolean {
  if (made.path === null) return false;
  const windows = /^[A-Za-z]:\//u.test(made.path) || made.path.startsWith("//");
  const wire = windows ? written.replaceAll("\\", "/") : written;
  const root = absolutePathRoot(wire);
  const parts: string[] = [];
  for (const part of wire.slice(root?.length ?? 0).split("/")) {
    if (part === "" || part === ".") continue;
    if (part !== "..") parts.push(part);
    else if (!root || parts.pop() === undefined) return false;
  }
  if (parts.length === 0) return false;
  const same = (a: string, b: string) => (windows ? a.toLowerCase() === b.toLowerCase() : a === b);
  if (root) return same(`${root}${parts.join("/")}`, made.path);
  const tail = `/${parts.join("/")}`;
  return made.path.length > tail.length && same(made.path.slice(-tail.length), tail);
}

/** The panels' first widths before the operator drags one, and their floors (B24), in CSS pixels. */
export const DESK_WIDTHS = { session: 500, view: 640, sessionMin: 360, viewMin: 360, chatMin: 420 } as const;

/**
 * The widths the room draws (B22-B24): the remembered ones, else the first
 * ones. The tab panel's view width gives way first, down to its floor; with
 * no room for both floors the tab panel covers Desk's place whole. A session
 * panel that would leave Desk's chat under its floor floats over the chat.
 */
export function roomWidths(available: number, stored: { view_width: number | null; session_width: number | null }, session: boolean): { session: number; view: number; panelCovers: boolean; sessionFloats: boolean } {
  const sessionWant = Math.max(DESK_WIDTHS.sessionMin, stored.session_width ?? DESK_WIDTHS.session);
  const sessionWidth = session ? Math.min(sessionWant, Math.max(DESK_WIDTHS.sessionMin, available)) : 0;
  const sessionFloats = session && available - sessionWidth < DESK_WIDTHS.chatMin;
  const left = available - sessionWidth;
  const viewWant = Math.max(DESK_WIDTHS.viewMin, stored.view_width ?? DESK_WIDTHS.view);
  const panelCovers = left < DESK_WIDTHS.viewMin;
  const view = panelCovers ? Math.max(0, left) : Math.min(viewWant, left);
  return { session: sessionWidth, view, panelCovers, sessionFloats };
}
