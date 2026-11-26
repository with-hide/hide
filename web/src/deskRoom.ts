// Desk's room in this window (PRD desk-2-ui D-01, D-05, D-12, D-13; PRD
// desk-own-desk D-02, D-03, D-05, D-16): which session is open beside
// Desk, and whether Desk's own desk shows, floating at that session's left
// edge, or at the window's right edge when none is open. The desk's tabs are
// the core's (`desk_view`, `deskDesk.ts`); the window keeps only whether it
// shows, whether it covers Desk's chat, and its Explorer column, all of which
// start closed every time the room is entered. The core keeps the two
// panels' widths (`desk.lane`).

import { ENDED_JOB_STATES, livePane } from "./desk";
import type { DeskItem, DeskJob, DeskMoved, DeskOutput } from "./snapshot";
import { absolutePathRoot, normalize, underFolder } from "./terminalLinks";

export type DeskRoomWindow = {
  /** The pane whose session is open beside Desk, null while the right side is closed (B1, B2). */
  session: string | null;
  /** Desk's desk is shown (B9); with no tab and no Explorer column there is nothing to draw (`deskShown`). */
  desk: boolean;
  /** The desk covers Desk's chat whole (`⤢`, B40). */
  expanded: boolean;
  /** The Explorer's column at the desk's left, the session's checkout (B37). */
  explorer: boolean;
  /** The session the last close took away in this visit of the room, which ⌘E opens again; the web's memory alone, the core keeps no open session (operator decision 2026-11-25). */
  closed: string | null;
  /** Moves each time `답하기` or `직접 쓰기` asks for the session's input box, so it takes the keyboard once per press. */
  answer: number;
};

export const DESK_ROOM_CLOSED: DeskRoomWindow = { session: null, desk: false, expanded: false, explorer: false, closed: null, answer: 0 };

/** Where the desk stands: attached to the session panel's left edge, or at the window's right edge with no session open (B16). */
export type DeskPanelPlace = "beside_session" | "window_edge";

export function panelPlace(room: DeskRoomWindow): DeskPanelPlace {
  return room.session !== null ? "beside_session" : "window_edge";
}

/** Whether Desk's desk is drawn: shown, and holding a tab or the Explorer's column (B6). */
export function deskShown(room: DeskRoomWindow, tabs: number): boolean {
  return room.desk && (tabs > 0 || (room.explorer && room.session !== null));
}

/**
 * A session opened beside Desk (B2, B3, B13, B36): Desk's desk is no
 * session's, so it stays as it was (PRD desk-own-desk D-03, D-14); another
 * session's checkout is not the Explorer's, so that column closes.
 */
export function openSession(room: DeskRoomWindow, pane: string, options: { answer?: boolean } = {}): DeskRoomWindow {
  const answer = options.answer ? room.answer + 1 : room.answer;
  if (room.session === pane) return { ...room, answer };
  return { ...room, session: pane, explorer: false, answer };
}

/**
 * What stands at Desk's right: the session open beside Desk, else 한눈에
 * unless the operator folded it (`desk.lane.glance_folded`), else nothing
 * and Desk's chat takes the room's width.
 */
export type RightSide = "session" | "glance" | null;

export function rightSide(room: DeskRoomWindow, folded: boolean): RightSide {
  if (room.session !== null) return "session";
  return folded ? null : "glance";
}

/** Something opened on Desk's desk: the desk shows with every tab it holds (B9-B11, B14). */
export function showDesk(room: DeskRoomWindow): DeskRoomWindow {
  return room.desk ? room : { ...room, desk: true };
}

/** `✕`, Escape or a press on the dimmed outside: the desk hides and its tabs stay (B18). */
export function hideDesk(room: DeskRoomWindow): DeskRoomWindow {
  return room.desk || room.expanded || room.explorer ? { ...room, desk: false, expanded: false, explorer: false } : room;
}

/** `⤢` at the tab strip's end (B40): the desk covers Desk's chat whole, or goes back to its width. */
export function toggleExpanded(room: DeskRoomWindow): DeskRoomWindow {
  return { ...room, expanded: !room.expanded };
}

/**
 * `파일 ⇧⌘E` (B37): the desk with the Explorer's column, or that column
 * closed when it shows; a desk left with no tab hides. Only a session's
 * checkout has an Explorer, so with no session it does nothing.
 */
export function toggleExplorer(room: DeskRoomWindow, tabs: number): DeskRoomWindow {
  if (room.session === null) return room;
  if (!room.desk || !room.explorer) return { ...room, desk: true, explorer: true };
  return tabs === 0 ? hideDesk(room) : { ...room, explorer: false };
}

/**
 * Where the room follows the sessions Desk's turns start (feedback 2, frame
 * J): the conversation it follows, the newest line it held when the room was
 * entered (nothing before it opens by itself, B26), and the turn whose first
 * new session was already settled, opened or left as a chip.
 */
export type NewSessionFollow = { generation: number; after: number; settled: number | null };

/** The follow a room starts with: every line already there is old. */
export function followFrom(generation: number, items: readonly DeskItem[]): NewSessionFollow {
  return { generation, after: items.at(-1)?.id ?? -1, settled: null };
}

/**
 * The open-beside rule of a session a Desk turn starts (frame J, 가-2, 나-1,
 * 다). A session Desk started is its turn's `job` line, the core's record of
 * the start, never a name read from the answer; a letter handed to a session
 * that already runs is a `handed` line and opens nothing. The turn's first
 * new session, once its pane exists, opens beside Desk when the right side
 * is empty, no session and no desk shown; when something shows it stays
 * and the session is only its chip. Either way the turn is settled, so its
 * other sessions, and the first one again after a close, stay chips.
 */
export function newSessionBeside(follow: NewSessionFollow, generation: number, items: readonly DeskItem[], cards: Readonly<Record<string, DeskJob>>, room: DeskRoomWindow): { follow: NewSessionFollow; open: string | null } {
  // A new conversation starts empty: every line in it is new.
  const now = follow.generation === generation ? follow : { generation, after: -1, settled: null };
  const turn = items.reduce((last, item) => (item.kind === "user" ? item.id : last), -1);
  if (now.settled === turn) return { follow: now, open: null };
  for (const item of items) {
    if (item.kind !== "job" || item.id <= now.after || item.id <= turn) continue;
    const card = cards[item.job_id];
    // A job the core has not listed yet, or one still waiting for its pane, holds the turn's first place.
    if (!card) return { follow: now, open: null };
    // A start that failed is no session: the next one is the turn's first, and a start retried under the same job is looked at again.
    // An ended job has no pane to open now: the turn's first session is the next job, unless this one starts again.
    if (ENDED_JOB_STATES.has(card.state)) continue;
    if (!card.pane_id) return { follow: now, open: null };
    const empty = room.session === null && !room.desk;
    return { follow: { ...now, settled: turn }, open: empty ? card.pane_id : null };
  }
  return { follow: now, open: null };
}

/**
 * The room once a gone session runs again in another pane (`desk_jobs.moved`,
 * one `이어서 하기` resumed): the session open beside Desk, and the one ⌘E
 * opens again, are that pane, so the panel's files, terminal and input reach
 * the live session.
 */
export function followMoved(room: DeskRoomWindow, moved: readonly DeskMoved[]): DeskRoomWindow {
  const session = room.session === null ? null : livePane(moved, room.session);
  const closed = room.closed === null ? null : livePane(moved, room.closed);
  return session === room.session && closed === room.closed ? room : { ...room, session, closed };
}

/** The session panel's `✕`, Escape or ⌘E (B6): Desk widens again and the session is kept for ⌘E to open again; Desk's desk stays as it was (B13). */
export function closeSession(room: DeskRoomWindow): DeskRoomWindow {
  if (!room.session) return room;
  return { ...room, session: null, explorer: false, closed: room.session };
}

/**
 * Whether `written`, a file as a session's call typed it, names the View
 * `made` opened (B16): only an absolute path, the same file once `.` and
 * `..` are read. A relative spelling names nothing, since the folder the
 * call read it from may not be the pane's (the agent can `cd` first), and a
 * file of the same name elsewhere would be taken for it. A View's path is
 * spelled as the wire spells it, `/` between names on every system, so a
 * Windows device's backslashes are read as `/` and its drive letter in
 * either case (docs/ARCHITECTURE.md, The platform layer).
 */
export function namesView(made: DeskOutput, written: string): boolean {
  if (made.path === null) return false;
  const windows = /^[A-Za-z]:\//u.test(made.path) || made.path.startsWith("//");
  const wire = windows ? written.replaceAll("\\", "/") : written;
  if (!absolutePathRoot(wire)) return false;
  const full = normalize(wire);
  return windows ? full.toLowerCase() === made.path.toLowerCase() : full === made.path;
}

/** An address that names its scheme, so it is the same page whatever folder the call ran in. */
const SCHEMED = /^[a-z][a-z0-9+.-]*:\/\//iu;

/**
 * The output a session's own line names, as its card lists it when it does. A line whose call's answer named its View
 * (`viewId`, the View id `hide browser|file|diff open` printed) is that View and no other. A line with no such answer
 * (a refused open, an older reader) is the card's output only when its words name one alone: the same id, an address
 * with its scheme, or an absolute path; a relative spelling or a host without a scheme stays the words it typed, since
 * reading it again could take another View for it (wrongly linked is worse than not linked).
 * A line the card does not list names a file as the session typed it, read where the session runs (B27): a relative
 * spelling under the session's own folder (`folder`, its pane's folder), and with that session's device and checkout
 * when they are another machine's; read without them it would go to this machine's boundary, which refuses a path of
 * another machine's checkout.
 */
export function outputNamed(listed: readonly DeskOutput[], output: DeskOutput, place: { device: string; checkout: string; folder: string } | null, node: string, viewId: string | null = null): DeskOutput {
  const made = viewId !== null
    ? listed.find((one) => one.id === viewId)
    : listed.find((one) => one.id === output.id || (output.url !== null && SCHEMED.test(output.url) && one.url === output.url) || (output.path !== null && namesView(one, output.path)));
  if (made) return made;
  if (!place || output.path === null || output.device_id !== null) return output;
  const path = absolutePathRoot(place.folder) ? underFolder(place.folder, output.path) : output.path;
  if (place.device !== node) return { ...output, path, device_id: place.device, checkout: place.checkout };
  return path === output.path ? output : { ...output, path };
}


/**
 * The panels' first widths before the operator drags one, and their floors
 * (B24), in CSS pixels. The desk's first width reaches left over the
 * sidebar until `panelMargin` of the window is left (Doop ③-다).
 */
export const DESK_WIDTHS = { session: 500, panelMargin: 140, sessionMin: 360, viewMin: 360, chatMin: 420 } as const;

/**
 * The widths the room draws (B22-B24): the remembered ones, else the first
 * ones. `available` is the room's own width and `reach` the window's width
 * left of it, the sidebar, which the floating desk may cover. The desk's
 * width gives way first, down to its floor; with no room for both floors the
 * desk covers Desk's place whole. A session panel that
 * would leave Desk's chat under its floor floats over the chat.
 */
export function roomWidths(available: number, reach: number, stored: { view_width: number | null; session_width: number | null }, session: boolean): { session: number; view: number; viewMax: number; panelCovers: boolean; sessionFloats: boolean } {
  const sessionWant = Math.max(DESK_WIDTHS.sessionMin, stored.session_width ?? DESK_WIDTHS.session);
  const sessionWidth = session ? Math.min(sessionWant, Math.max(DESK_WIDTHS.sessionMin, available)) : 0;
  const sessionFloats = session && available - sessionWidth < DESK_WIDTHS.chatMin;
  const left = available - sessionWidth;
  const viewMax = Math.max(0, left + reach);
  const viewWant = Math.max(DESK_WIDTHS.viewMin, stored.view_width ?? viewMax - DESK_WIDTHS.panelMargin);
  const panelCovers = left < DESK_WIDTHS.viewMin;
  const view = panelCovers ? Math.max(0, left) : Math.min(viewWant, viewMax);
  return { session: sessionWidth, view, viewMax, panelCovers, sessionFloats };
}
