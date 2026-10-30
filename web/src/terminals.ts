// One xterm.js instance per attached pane, keyed by pane id.
//
// An instance outlives the tab it is drawn in (PRD S2 D-05, amendment 1):
// the core keeps the panes of the last `ATTACHED_TAB_LIMIT` tabs attached and
// streams their output, so their instances stay alive, fed, and parked out of
// view while another tab is shown. Coming back to a tab shows the last frame
// at once; nothing is re-requested unless the size changed. An instance is
// disposed only when the core reports the pane released or no longer lists
// it (`retainTerminals`), never on a tab switch.
//
// The wheel is Herdr's too (PRD S2 B19): the pane has no local scrollback,
// so a wheel batch becomes one `terminal_scroll` per animation frame and the
// core answers with the viewport it now shows.
//
// A pointer gesture follows the routing policy (PRD S2 B20): a drag
// selects locally, and a single primary click that never
// dragged is replayed on release as one `terminal_click` with the pressed
// cell; the core decides whether the program gets a mouse report. ⌥ + press
// and a multi-click stay local, and so does a press on a link, whose click is
// the link's (`terminalLinkProvider.ts`). A copy of the selection is
// assembled here (`selection.ts`), not by xterm.

import { FitAddon } from "@xterm/addon-fit";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { Terminal, type IDisposable, type ITheme } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { submitExplorerImage, submitFiles } from "./attachments";
import { copiedExplorerFile, isImageDrag, terminalPathText, type ExplorerDrag } from "./explorerDrag";
import { useUiStore } from "./ui";
import { localDeviceId } from "./snapshot";
import { keySystem } from "./host";
import { terminalKey } from "./keys";
import { noteWriteComplete, probeEnabled } from "./probe";
import { loadTerminalRenderer } from "./terminalRenderer";
import { holdTerminalFont, setTerminalFontFamily, watchPixelRatio, type TerminalFont } from "./terminalFonts";
import { remoteControl, remoteTargetOfPane } from "./remote";
import { keyPayload, noteTerminalFocus } from "./keyTarget";
import { bufferRow, followScroll, movedSelection, selectionToText, type CellRow } from "./selection";
import { osc8Handler, registerTerminalLinks, terminalLinkAtCell, terminalPathOwner, type LinkState, type TerminalContextLink, type TerminalLinkActions } from "./terminalLinkProvider";
import { isHtmlFile } from "./browserViews";
import { pointerModifiers, wheelRows } from "./wheel";
import { useShellStore, type TerminalChunk } from "./store";
import { outsideFilesShown } from "./externalFiles";
import { catalogWorkspaces, type SnapshotRest } from "./snapshot";
import type { DispatchFn } from "./ws";
import type { ClipboardContent } from "./clipboard";

/** What a terminal asks of the shell's actions: where a clicked link goes, and the clipboard its copy and paste use. */
export type TerminalActions = TerminalLinkActions & {
  copyText(text: string, what: string): Promise<boolean>;
  readClipboard(what: string): Promise<ClipboardContent | null>;
};

type Instance = {
  /** Mutable routing address; Herdr may rekey it while retaining the terminal. */
  paneId: string;
  identity: string | null;
  term: Terminal;
  fit: FitAddon;
  /** Lives with the instance, including while parked; no retry after fallback. */
  renderer: IDisposable | null;
  dispatch: DispatchFn;
  /** Where a clicked link goes and the clipboard copy and paste use; replaced on every attach, like `dispatch`. */
  actions: TerminalActions;
  /** Whether the pointer is over a link. */
  link: LinkState;
  scale: number;
  /** The fonts for its size, held while it lives (`terminalFonts.ts`). */
  font: TerminalFont;
  /** The element the terminal was opened in; it moves between a host and the parking lot. */
  element: HTMLDivElement;
  /** The pane host it is shown in, or null while parked. */
  host: HTMLElement | null;
  observer: ResizeObserver | null;
  /** A self-contained snapshot arrived while parked; the next show asks for a full frame. */
  stale: boolean;
  /** The grid the core last got from this instance, so a host resize that keeps it sends nothing; null when the core may not have one. */
  grid: { cols: number; rows: number } | null;
  /** Wheel rows accumulated toward the next flush, and the pending flush. */
  wheel: { held: boolean; rows: number; remainder: number; column: number; row: number; modifiers: number; frame: number | null };
  /**
   * The selection a wheel scrolled, including its original offscreen ends.
   * Retained while clipped so a reverse wheel restores the complete range.
   */
  follow: FollowedSelection | null;
  /**
   * The original copy text while a selection is partly or wholly offscreen.
   */
  kept: string | null;
  /** The cell of a primary press that may still become a click, or null once it dragged or was not one. */
  press: { column: number; row: number } | null;
  /**
   * An input method just finished a composition and xterm has not sent it
   * yet: it sends the composed text one task after `compositionend`, once the
   * textarea holds the final syllable (its `CompositionHelper`).
   */
  commitPending: boolean;
  /** Keys the custom handler took that wait behind such a send, in order. */
  keysWaiting: number;
  disposeHandlers: () => void;
};

/** A selection as it stood when the wheel scrolled, with the rows it covered. */
type FollowedSelection = {
  start: { x: number; y: number };
  end: { x: number; y: number };
  /** The clipped range xterm currently draws, separate from the original ends. */
  drawn: { start: { x: number; y: number }; end: { x: number; y: number } };
  rows: string[];
  /** What Copy takes from it. */
  text: string;
  /** The shift of the text the wheel asked for, rows, positive moving it down. */
  expected: number;
};

const instances = new Map<string, Instance>();

let identityCatalog: SnapshotRest | null | undefined;
const identities = new Map<string, string>();
let reconciledCatalog: SnapshotRest | null | undefined;

function terminalIdentity(paneId: string): string | null {
  const rest = useShellStore.getState().rest;
  if (identityCatalog !== rest) {
    identityCatalog = rest;
    identities.clear();
    for (const workspace of catalogWorkspaces(rest)) {
      for (const checkout of workspace.checkouts) {
        for (const tab of checkout.tabs) {
          for (const pane of tab.panes) {
            if (pane.terminal_id) identities.set(pane.id, JSON.stringify([workspace.device_id, pane.terminal_id]));
          }
        }
      }
    }
  }
  return identities.get(paneId) ?? null;
}

/** Readdress once per snapshot, before an old host parks or a new one attaches. */
function reconcileTerminalIds() {
  const rest = useShellStore.getState().rest;
  if (reconciledCatalog === rest) return;
  reconciledCatalog = rest;
  const attached = new Set(attachedPaneIds());
  const addresses = new Map<string, string | null>();
  for (const paneId of attached) {
    const identity = terminalIdentity(paneId);
    if (identity) addresses.set(identity, addresses.has(identity) ? null : paneId);
  }
  for (const [previous, instance] of instances) {
    instance.identity ??= terminalIdentity(previous);
    if (!instance.identity || attached.has(previous)) continue;
    const current = addresses.get(instance.identity);
    if (!current || instances.has(current)) continue;
    instances.delete(previous);
    instance.paneId = current;
    instance.element.dataset.terminal = current;
    instances.set(current, instance);
  }
}

/** One inert image of a cached terminal, without fitting or attaching its PTY. */
export function mountTerminalPreview(paneId: string, host: HTMLElement): boolean {
  reconcileTerminalIds();
  const instance = instances.get(paneId);
  if (!instance) return false;
  const canvases = [...instance.element.querySelectorAll("canvas")];
  if (canvases.reduce((pixels, canvas) => pixels + canvas.width * canvas.height, 0) > 16_777_216) return false;
  const clone = instance.element.cloneNode(true) as HTMLElement;
  const copies = [...clone.querySelectorAll("canvas")];
  for (const [index, canvas] of canvases.entries()) {
    const copy = copies[index];
    if (!copy) return false;
    const context = copy.getContext("2d");
    if (!context) return false;
    context.drawImage(canvas, 0, 0);
  }
  clone.setAttribute("aria-hidden", "true");
  clone.inert = true;
  clone.style.pointerEvents = "none";
  clone.removeAttribute("data-terminal");
  for (const element of clone.querySelectorAll("[id]")) element.removeAttribute("id");
  host.replaceChildren(clone);
  return true;
}

/** Keys that end nothing by themselves: the chord they begin may be Copy. */
const MODIFIER_KEYS = new Set(["Meta", "Control", "Shift", "Alt", "AltGraph", "CapsLock", "Fn", "OS"]);

/** The most lines one `terminal_scroll` carries (the core reads a u16). */
const SCROLL_LINES_MAX = 65535;

let parking: HTMLDivElement | null = null;

/** A hidden lot in the document, so a parked terminal keeps its canvas and layout state. */
function parkingLot(): HTMLDivElement {
  if (!parking) {
    parking = document.createElement("div");
    parking.hidden = true;
    parking.dataset.terminalParking = "true";
    document.body.append(parking);
  }
  return parking;
}

function token(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

const ANSI = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"] as const;

/** The terminal's colors from the current theme's tokens, ANSI 16 included. */
function terminalTheme(): ITheme {
  const theme: Record<string, string> = {
    background: token("--background"),
    foreground: token("--foreground"),
    cursor: token("--foreground"),
    cursorAccent: token("--background"),
    selectionBackground: token("--terminal-selection"),
  };
  for (const name of ANSI) {
    theme[name] = token(`--terminal-${name}`);
    theme[`bright${name[0]?.toUpperCase()}${name.slice(1)}`] = token(`--terminal-bright-${name}`);
  }
  return theme;
}

/**
 * Re-color every terminal after a theme change. The instances are kept, so a
 * pane's content, scroll position and a half-typed line stay (B13).
 */
export function applyTerminalTheme() {
  const theme = terminalTheme();
  for (const instance of instances.values()) instance.term.options.theme = theme;
}

function tokenPx(name: string): number {
  const value = Number.parseFloat(token(name));
  if (!Number.isFinite(value)) throw new Error(`token ${name} is not a length`);
  return value;
}

/** Appearance's Terminal text size, or null for the token's until the core reports one. */
let baseFontSize: number | null = null;

function fontSizeAt(scale: number): number {
  return Math.round((baseFontSize ?? tokenPx("--text-terminal-base")) * scale);
}

/**
 * Applies Appearance's Terminal text size to every terminal, each still
 * multiplied by its own pane's scale; a pane whose size moved sends its new
 * fit as `terminal_resize`, as a pane's own zoom does.
 */
export function applyTerminalFontSize(size: number | null) {
  if (baseFontSize === size) return;
  baseFontSize = size;
  warmTerminalFonts();
  for (const instance of instances.values()) {
    if (instance.term.options.fontSize === fontSizeAt(instance.scale)) continue;
    applyFont(instance, false);
  }
}

/**
 * Applies General's Terminal font, the installed font every terminal draws
 * in ahead of the `--font-mono` token; empty is the token alone. A pane
 * whose cell it changes sends its new fit.
 */
export function applyTerminalFontFamily(family: string) {
  if (!setTerminalFontFamily(family)) return;
  warmTerminalFonts();
  for (const instance of instances.values()) applyFont(instance, true);
}

/** Fits the fallbacks for the base size before a terminal asks for them, so the first one draws its icons and CJK at once. */
function warmTerminalFonts() {
  holdTerminalFont(fontSizeAt(1), token("--font-mono")).release();
}

let pixelRatioWatched = false;

/** The fallbacks are fitted in device pixels, so another display or the browser's zoom fits them again. */
function watchFontPixelRatio() {
  if (pixelRatioWatched) return;
  pixelRatioWatched = true;
  watchPixelRatio(() => {
    for (const instance of instances.values()) applyFont(instance, true);
  });
}

/**
 * Points the terminal at the fonts for its size: it holds the fitted set for
 * that size, the Terminal font and the pixel ratio, draws in its family, and
 * sends the fit (only a changed one with `onlyChanged`).
 */
function applyFont(instance: Instance, onlyChanged: boolean) {
  const size = fontSizeAt(instance.scale);
  const previous = instance.font;
  instance.font = holdTerminalFont(size, token("--font-mono"));
  previous.release();
  instance.term.options.fontSize = size;
  instance.term.options.fontFamily = instance.font.family();
  sendGrid(instance.paneId, instance, false, onlyChanged);
  followFont(instance);
}

/**
 * Once the fallbacks for the held size are fitted, draws in them. xterm
 * measures its cell again on the family change, so the fit that goes out
 * is measured with every font loaded; only a changed grid is sent.
 */
function followFont(instance: Instance) {
  const font = instance.font;
  void font.ready.then(() => {
    if (instance.font !== font || instances.get(instance.paneId) !== instance) return;
    if (instance.term.options.fontFamily === font.family()) return;
    instance.term.options.fontFamily = font.family();
    sendGrid(instance.paneId, instance, false, true);
  });
}

function decodeChunk(chunk: TerminalChunk): Uint8Array | null {
  if (!chunk.bytes_base64) return null;
  const binary = atob(chunk.bytes_base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

function bytesBase64(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

export function feedChunks(chunks: TerminalChunk[]) {
  reconcileTerminalIds();
  for (const chunk of chunks) {
    const instance = instances.get(chunk.pane_id);
    if (!instance) continue;
    const bytes = decodeChunk(chunk);
    if (!bytes) continue;
    if (probeEnabled()) {
      const term = instance.term;
      term.write(bytes, () => {
        followSelection(instance);
        noteWriteComplete(term);
      });
    } else {
      instance.term.write(bytes, instance.follow?.expected ? () => followSelection(instance) : undefined);
    }
  }
}

/** A self-contained snapshot may follow a core restart; every pane redraws from its next full frame. */
export function resetAllTerminals() {
  for (const instance of instances.values()) {
    instance.term.reset();
    setKept(instance, null);
    instance.grid = null;
    if (!instance.host) instance.stale = true;
  }
}

export function terminalFor(paneId: string | null): Terminal | null {
  return paneId ? (instances.get(paneId)?.term ?? null) : null;
}

/**
 * True while `focusTerminal` moves DOM focus to follow the snapshot, so the
 * textarea's focus listener does not report the core's own move back to it
 * as an operator action; that echo, met by Herdr's later confirmation of the
 * previous move, kept two panes trading focus on a slow runner.
 */
let followingSnapshot = false;

/** Moves keyboard focus to the pane the snapshot names; not an operator action. */
/** Whether the pane's terminal asked for bracketed paste, which the core
 * wraps the attachment token in. */
export function bracketedPaste(paneId: string): boolean {
  return terminalFor(paneId)?.modes.bracketedPasteMode ?? false;
}

export function focusTerminal(paneId: string) {
  const instance = instances.get(paneId);
  if (!instance) return;
  followingSnapshot = true;
  try {
    instance.term.focus();
  } finally {
    followingSnapshot = false;
  }
}

/**
 * Gives keyboard focus back after a sheet or dialog closes. A terminal gets it
 * through `focusTerminal` on the pane the core says is focused, never by
 * re-focusing the textarea that held it: that focus would be reported as the
 * operator moving to that pane, and undo a move the dialog itself caused (a
 * created worktree's pane). Any other element simply takes focus back.
 */
export function restoreFocus(previous: HTMLElement | null) {
  if (previous?.closest(".xterm")) {
    const paneId = useShellStore.getState().focusedPaneId;
    if (paneId) focusTerminal(paneId);
    return;
  }
  if (previous?.isConnected) previous.focus();
}

/** Pane ids that currently hold an instance, shown or parked (measurement and e2e seam). */
export function liveTerminalIds(): string[] {
  return [...instances.keys()];
}

/** Pane ids the core streams right now: listed in the terminal section and not released. */
export function attachedPaneIds(): string[] {
  return (useShellStore.getState().rest?.terminal?.panes ?? [])
    .filter((pane) => pane.transport_state !== "released")
    .map((pane) => pane.pane_id);
}

function disposeInstance(paneId: string, instance: Instance) {
  instance.disposeHandlers();
  instance.font.release();
  instance.renderer?.dispose();
  instance.term.dispose();
  instance.element.remove();
  instances.delete(paneId);
}

/**
 * Fits the terminal to its host and sends the grid out when it changed. The
 * grid follows the rectangle drawn for the pane the moment it is drawn,
 * whether or not Herdr has confirmed the layout behind it (PRD
 * grid-ahead-of-herdr D-03): the PTY size is this controller's to set, so a
 * layout Herdr draws differently only moves the host once more and the
 * observer sends that fit too.
 */
function sendGrid(paneId: string, instance: Instance, newView: boolean, onlyChanged = false) {
  if (!instance.host) return;
  // The rows a selection was remembered at no longer exist at another grid.
  instance.follow = null;
  instance.fit.fit();
  const { term } = instance;
  if (term.cols < 2 || term.rows < 2) {
    // A host not laid out yet sent nothing, so its real size must go out.
    instance.grid = null;
    return;
  }
  if (onlyChanged && instance.grid?.cols === term.cols && instance.grid.rows === term.rows) return;
  instance.grid = { cols: term.cols, rows: term.rows };
  instance.dispatch({
    schema_version: 2,
    kind: "terminal_viewport",
    payload: { pane_id: paneId, cols: term.cols, rows: term.rows, new_view: newView },
  });
  instance.dispatch({
    schema_version: 2,
    kind: "terminal_resize",
    payload: { pane_id: paneId, cols: term.cols, rows: term.rows },
  });
}

/** Asks the core for a full frame of the pane; used after every self-contained snapshot. */
export function requestView(paneId: string) {
  const instance = instances.get(paneId);
  if (!instance) return;
  instance.stale = false;
  sendGrid(paneId, instance, true);
}

/** Applies the core's per-pane text scale; the new fit goes out as `terminal_resize` (PRD S2 B13). */
export function setTextScale(paneId: string, scale: number) {
  const instance = instances.get(paneId);
  if (!instance || instance.scale === scale) return;
  instance.scale = scale;
  applyFont(instance, false);
}

function screenRows(term: Terminal): string[] {
  const buffer = term.buffer.active;
  const rows: string[] = [];
  for (let y = 0; y < term.rows; y += 1) rows.push(buffer.getLine(y)?.translateToString(true) ?? "");
  return rows;
}

/**
 * The pane has no local scrollback: Herdr redraws its viewport in place, so
 * xterm keeps the selection on the same screen cells while the text under it
 * moves. A wheel remembers what the selection covers, and the frame that
 * answers it moves the selection to where that text went, or to the part
 * still on screen; a selection whose text left the screen is cleared rather
 * than left on other text.
 */
function armFollow(instance: Instance, rows: number) {
  const { term } = instance;
  const range = term.getSelectionPosition();
  if (!range) {
    instance.follow = null;
    return;
  }
  const known = instance.follow;
  const same = known && known.drawn.start.x === range.start.x && known.drawn.start.y === range.start.y && known.drawn.end.x === range.end.x && known.drawn.end.y === range.end.y;
  const shift = rows > 0 ? -Math.abs(rows) : Math.abs(rows);
  if (same) {
    known.expected += shift;
    return;
  }
  // A selection ending at column 0 does not take that row.
  const last = range.end.x === 0 && range.end.y > range.start.y ? range.end.y : range.end.y + 1;
  instance.follow = {
    start: { ...range.start },
    end: { ...range.end },
    drawn: range,
    rows: screenRows(term).slice(range.start.y, last),
    text: selectedText(instance) ?? "",
    expected: shift,
  };
}

/** Nothing of the selection is on screen any more: Copy keeps its text, the highlight goes. */
function keepOffscreen(instance: Instance, follow: FollowedSelection) {
  instance.term.clearSelection();
  setKept(instance, follow.text === "" ? null : follow.text);
}

/**
 * Keeps or drops the text of a selection clipped by the screen. The terminal's
 * input textarea holds it selected while kept, since the browser fires a copy
 * event (⌘C, the Edit menu) only for a selection to copy; `onCopy` then puts
 * the text on the clipboard.
 */
function setKept(instance: Instance, text: string | null) {
  if (instance.kept === text) return;
  instance.kept = text;
  const area = instance.term.textarea;
  if (!area) return;
  area.value = text ?? "";
  if (text !== null && document.activeElement === area) area.select();
}

/** The terminal's textarea was focused: xterm empties it on blur, so the kept text is put back. */
function holdKept(instance: Instance) {
  const area = instance.term.textarea;
  if (!area || instance.kept === null) return;
  area.value = instance.kept;
  area.select();
}

/** Whether Copy has something to take: a drawn selection or one the wheel carried off. */
function hasCopyText(instance: Instance): boolean {
  return instance.term.hasSelection() || instance.kept !== null;
}

/**
 * Runs after a frame is drawn: moves a waiting selection onto the text it
 * covered. It waits while the wheel's rows are not all accounted for, so
 * several frames of one fast scroll each move it. A clipped selection keeps
 * its original ends after the move, until restored, cleared or replaced.
 */
function followSelection(instance: Instance) {
  const follow = instance.follow;
  if (!follow || follow.expected === 0) return;
  const { term } = instance;
  const range = term.getSelectionPosition();
  if (!range || range.start.x !== follow.drawn.start.x || range.start.y !== follow.drawn.start.y || range.end.x !== follow.drawn.end.x || range.end.y !== follow.drawn.end.y) {
    instance.follow = null;
    return;
  }
  // Only the selected rows are read while they are unchanged, which is every
  // chunk of output elsewhere on the screen.
  const buffer = term.buffer.active;
  const unchanged = follow.rows.every((text, index) => {
    const row = follow.start.y + index;
    return row < 0 || row >= term.rows || (buffer.getLine(row)?.translateToString(true) ?? "") === text;
  });
  if (unchanged) return;
  const screen = screenRows(term);
  const shift = followScroll(follow.rows, follow.start.y, screen, follow.expected);
  if (shift === null) {
    instance.follow = null;
    // Text the wheel carried off the screen leaves nothing to select; text
    // that changed in place under a selection stays as xterm keeps it.
    const first = follow.start.y + follow.expected;
    const last = follow.end.y + follow.expected;
    if (last < 0 || first > term.rows - 1) keepOffscreen(instance, follow);
    return;
  }
  const moved = movedSelection(follow.start, follow.end, shift, term.rows, term.cols);
  if (!moved) {
    instance.follow = null;
    keepOffscreen(instance, follow);
    return;
  }
  term.select(moved.column, moved.row, moved.length);
  const now = term.getSelectionPosition();
  const remaining = follow.expected - shift;
  if (!now) {
    instance.follow = null;
    return;
  }
  follow.start.y += shift;
  follow.end.y += shift;
  follow.drawn = now;
  follow.expected = Math.sign(remaining) === Math.sign(follow.expected) ? remaining : 0;
  const clipped = follow.start.y < 0 || follow.end.y >= term.rows;
  setKept(instance, clipped ? follow.text : null);
  if (!clipped && follow.expected === 0) instance.follow = null;
}

function flushWheel(paneId: string, instance: Instance) {
  const wheel = instance.wheel;
  wheel.frame = null;
  const rows = wheel.rows;
  wheel.rows = 0;
  if (rows === 0) return;
  // A wheel turned while the button is held belongs to the drag, which
  // `select` would end; the selection then stays as xterm keeps it.
  if (wheel.held) instance.follow = null;
  else armFollow(instance, rows);
  instance.dispatch({
    schema_version: 2,
    kind: "terminal_scroll",
    payload: {
      pane_id: paneId,
      // A DOM deltaY grows downward while AppKit's scrollingDeltaY grows
      // upward, so this ternary reads the DOM sign, not AppKit's; Herdr's
      // `up` shows older lines. The core's line count is
      // a u16, so a scripted burst is clamped rather than refused.
      direction: rows > 0 ? "down" : "up",
      lines: Math.min(Math.abs(rows), SCROLL_LINES_MAX),
      column: wheel.column,
      row: wheel.row,
      modifiers: wheel.modifiers,
    },
  });
}

function onWheel(paneId: string, instance: Instance, event: WheelEvent) {
  // The wheel has one owner. Left to xterm, a buffer without scrollback
  // (ours always, and any alternate-screen program) turns the same event
  // into cursor-key bytes on the PTY, and a mouse-tracking program gets
  // xterm's own wheel report; Herdr decides both from terminal_scroll.
  event.stopImmediatePropagation();
  // ⌥ + wheel stays with the browser: nothing to Herdr and nothing to
  // the PTY.
  if (event.altKey) return;
  event.preventDefault();
  const geometry = cellGeometry(instance);
  if (!geometry) return;
  const wheel = instance.wheel;
  const moved = wheelRows(event.deltaY, event.deltaMode, geometry.rowHeight, wheel.remainder);
  wheel.remainder = moved.remainder;
  wheel.rows += moved.rows;
  const cell = cellAt(geometry, event);
  wheel.column = cell.column;
  wheel.row = cell.row;
  wheel.modifiers = pointerModifiers(event);
  wheel.held = (event.buttons & 1) !== 0;
  if (wheel.frame === null) wheel.frame = requestAnimationFrame(() => flushWheel(paneId, instance));
}

type CellGeometry = { rect: DOMRect; cols: number; rows: number; colWidth: number; rowHeight: number };

/**
 * The shown pane's cell grid, or null while parked. The grid is read from
 * xterm's own screen element, which is exactly `cols` by `rows` cells: the
 * host is larger by the fit's remainder, so a cell derived from the host
 * drifts by up to one row and one column toward the far edge, which is where
 * a one-row button in a full-screen program sits.
 */
function cellGeometry(instance: Instance): CellGeometry | null {
  if (!instance.host) return null;
  const { term } = instance;
  const screen = instance.element.querySelector(".xterm-screen");
  if (!screen) return null;
  const rect = screen.getBoundingClientRect();
  return {
    rect,
    cols: term.cols,
    rows: term.rows,
    colWidth: term.cols > 0 ? rect.width / term.cols : 0,
    rowHeight: term.rows > 0 ? rect.height / term.rows : 0,
  };
}

/** The zero-based cell under the pointer, clamped to the grid. */
function cellAt(geometry: CellGeometry, event: { clientX: number; clientY: number }): { column: number; row: number } {
  const { rect, cols, rows, colWidth, rowHeight } = geometry;
  return {
    column: colWidth > 0 ? Math.max(0, Math.min(cols - 1, Math.floor((event.clientX - rect.left) / colWidth))) : 0,
    row: rowHeight > 0 ? Math.max(0, Math.min(rows - 1, Math.floor((event.clientY - rect.top) / rowHeight))) : 0,
  };
}

function onMouseDown(instance: Instance, event: MouseEvent) {
  instance.press = null;
  // A right-click opens the menu, whose Copy still takes the kept text.
  // On macOS a ctrl-press is a right-click too.
  if (event.button === 0 && !(event.ctrlKey && keySystem() === "mac")) {
    instance.follow = null;
    setKept(instance, null);
  }
  // The primary button alone can become a click; ⌥ + press and a second
  // click of a multi-click are the `application` and `localSelection`
  // routes, and neither is replayed to the program. A press on a link is the
  // link's click, so the program does not hear it too.
  if (event.button !== 0 || event.altKey || event.detail !== 1 || instance.link.hovered) return;
  const geometry = cellGeometry(instance);
  if (!geometry) return;
  instance.press = cellAt(geometry, event);
}

function onMouseUp(paneId: string, instance: Instance, event: MouseEvent) {
  const press = instance.press;
  instance.press = null;
  if (!press || event.button !== 0) return;
  const geometry = cellGeometry(instance);
  if (!geometry) return;
  const release = cellAt(geometry, event);
  // Leaving the pressed cell, or a selection xterm made on the way, is a drag.
  if (release.column !== press.column || release.row !== press.row || instance.term.hasSelection()) return;
  instance.dispatch({
    schema_version: 2,
    kind: "terminal_click",
    payload: { pane_id: paneId, column: press.column, row: press.row, modifiers: pointerModifiers(event) },
  });
}

/** The text the current drag selection copies, or null when nothing is selected. */
function selectedText(instance: Instance): string | null {
  const { term } = instance;
  const range = term.getSelectionPosition();
  if (!range) return null;
  const buffer = term.buffer.active;
  const rows: CellRow[] = [];
  for (let y = range.start.y; y <= range.end.y; y += 1) rows.push(bufferRow(buffer.getLine(y), term.cols));
  return selectionToText(rows, range.start.x, range.end.x);
}

/** The copy text of the pane's selection, for the probe; null without a selection or an instance. */
export function terminalSelectionText(paneId: string): string | null {
  const instance = instances.get(paneId);
  return instance ? (instance.kept ?? selectedText(instance)) : null;
}

/** Resolves only the cell right-clicked, using the link provider's existing bounds and cache. */
export async function terminalLinkAt(paneId: string, point: { clientX: number; clientY: number }): Promise<TerminalContextLink | null> {
  const instance = instances.get(paneId);
  const geometry = instance && cellGeometry(instance);
  if (!instance || !geometry || point.clientX < geometry.rect.left || point.clientX >= geometry.rect.right
    || point.clientY < geometry.rect.top || point.clientY >= geometry.rect.bottom) return null;
  const cell = cellAt(geometry, point);
  const link = await terminalLinkAtCell(instance.term, paneId, instance.link, cell.column + 1, instance.term.buffer.active.viewportY + cell.row + 1);
  if (link?.target.kind !== "path" || link.found?.kind !== "file" || !isHtmlFile(link.found.real)) return link;
  const owner = await terminalPathOwner(link.found);
  // An HTML file's source opens in Hide inside a checkout, and outside every
  // checkout where this window shows outside files (PRD outside-path-in-hide B9).
  return { ...link, inCheckout: !!owner, sourceInHide: !!owner || (owner === null && outsideFilesShown(useShellStore.getState().rest)) };
}

/** Pastes text as typed input, wrapped in bracketed paste when the program asked for it. */
export function pasteText(paneId: string, text: string) {
  instances.get(paneId)?.term.paste(text);
}

/** Types an agent reference (`agentReference.ts`) as input, with a space to keep typing after it and never Enter. */
export function typeReference(paneId: string, text: string) {
  pasteText(paneId, `${text} `);
}

/** Explorer Copy retains file identity only while the system text still matches. */
export function pasteExplorerCopy(paneId: string, text: string): boolean {
  const held = useUiStore.getState().explorerClipboard;
  const item = copiedExplorerFile(held, text);
  if (!item) {
    if (held?.terminalCopy) useUiStore.getState().setExplorerClipboard({ ...held, terminalCopy: false });
    return false;
  }
  pasteExplorerFile(paneId, item);
  return true;
}

/** Drag and Copy share one path/image decision and the existing terminal ingress. */
export function pasteExplorerFile(paneId: string, item: ExplorerDrag): void {
  const state = useShellStore.getState();
  if (item.device !== (remoteTargetOfPane(state.rest, paneId) ?? localDeviceId(state.rest))) {
    state.setAttachmentRefusal({ pane_id: paneId, reason: "source_device" });
    return;
  }
  if (isImageDrag(item)) void submitExplorerImage(paneId, item, bracketedPaste(paneId));
  else {
    try { pasteText(paneId, terminalPathText(item.path)); }
    catch { state.setAttachmentRefusal({ pane_id: paneId, reason: "invalid_request_id" }); }
  }
}

/**
 * Puts the pane's selection on the clipboard, for the menu's Copy and the
 * copy chord alike; a refused write is a diagnostic and the answer is false,
 * so the caller keeps the selection.
 */
export async function copySelection(paneId: string, where: string): Promise<boolean> {
  const text = terminalSelectionText(paneId);
  const instance = instances.get(paneId);
  if (text === null || !instance) return false;
  return instance.actions.copyText(text, `${where} selection`);
}

/**
 * Pastes the clipboard into the pane, for the menu's Paste and the paste
 * chord alike: an image as an attachment, the way ⌘V of one is, and text
 * as typed input, in bracketed paste when the program asked for it.
 */
export async function pasteClipboard(paneId: string, where: string): Promise<void> {
  const content = await instances.get(paneId)?.actions.readClipboard(where);
  if (!content) return;
  if (content.kind === "image") return void submitFiles(paneId, [content.file], true, bracketedPaste(paneId));
  if (content.text && !pasteExplorerCopy(paneId, content.text)) pasteText(paneId, content.text);
}

/** Selects the pane's whole text, for Copy to take; the pane keeps no scrollback, so that is what it shows. */
export function selectAllText(paneId: string) {
  const instance = instances.get(paneId);
  if (!instance) return;
  instance.follow = null;
  instance.term.selectAll();
}

function onCopy(instance: Instance, event: ClipboardEvent) {
  const text = instance.kept ?? selectedText(instance);
  if (text === null || !event.clipboardData) return;
  // One owner again: xterm's own copy handler would put its padded,
  // hard-broken rows on the clipboard from the same event.
  event.stopImmediatePropagation();
  event.preventDefault();
  event.clipboardData.setData("text/plain", text);
}

function createInstance(paneId: string, dispatch: DispatchFn, actions: TerminalActions, scale: number): Instance {
  const element = document.createElement("div");
  // Its own stacking context: xterm gives its layers z-indexes up to 11 (the
  // WebGL canvases, the helpers, the scrollbar), which would otherwise paint
  // over, and take the clicks of, whatever a pane draws on top of its terminal
  // (issue 1279).
  element.className = "absolute inset-0 isolate";
  element.dataset.terminal = paneId;
  const link: LinkState = { hovered: false };
  watchFontPixelRatio();
  const font = holdTerminalFont(fontSizeAt(scale), token("--font-mono"));
  const term = new Terminal({
    fontFamily: font.family(),
    fontSize: fontSizeAt(scale),
    theme: terminalTheme(),
    // Herdr owns the history; the pane shows the viewport the core sends.
    scrollback: 0,
    allowProposedApi: true,
    // A program's own links open like the ones found in the text, never
    // through xterm's confirm dialog.
    linkHandler: osc8Handler(() => instance.paneId, link, () => instance.actions, () => instance.term.hasSelection()),
    // A right-click opens the pane's menu; it leaves the drag selection as it
    // was, so Copy there copies what the operator selected, not a word.
    rightClickSelectsWord: false,
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  // Herdr lays its frames out with current Unicode widths; xterm's default
  // Unicode 6 tables give many emoji one cell where Herdr gave two.
  term.loadAddon(new Unicode11Addon());
  term.unicode.activeVersion = "11";
  const instance: Instance = {
    paneId,
    identity: terminalIdentity(paneId),
    term,
    fit,
    renderer: null,
    dispatch,
    actions,
    link,
    scale,
    font,
    element,
    host: null,
    observer: null,
    stale: false,
    grid: null,
    wheel: { held: false, rows: 0, remainder: 0, column: 0, row: 0, modifiers: 0, frame: null },
    follow: null,
    kept: null,
    press: null,
    commitPending: false,
    keysWaiting: 0,
    disposeHandlers: () => {},
  };
  // Right after a new tab or split was asked for, keys go to that request's
  // pane rather than this one (`keyTarget.ts`).
  const send = (bytes: Uint8Array) =>
    instance.dispatch({
      schema_version: 2,
      kind: "key",
      payload: keyPayload(instance.paneId, bytesBase64(bytes)),
    });
  const system = keySystem();
  term.attachCustomKeyEventHandler((event) => {
    if (event.type !== "keydown") return true;
    const action = terminalKey(event, system, hasCopyText(instance));
    // A key that is not a modifier ends the text kept off screen, as it ends
    // a drawn selection; the chord that copies it keeps it for its own read.
    // On macOS ⌘C is the browser's own copy, which `terminalKey` leaves alone.
    const copying = action?.kind === "copy"
      || (system === "mac" && event.metaKey && !event.shiftKey && !event.altKey && (event.code === "KeyC" || event.key.toLowerCase() === "c"));
    if (!copying && !MODIFIER_KEYS.has(event.key)) {
      instance.follow = null;
      setKept(instance, null);
    }
    if (!action) return true;
    event.preventDefault();
    const run = () => {
      if (action.kind === "bytes") send(action.bytes);
      // The page's own Ctrl+Shift+V is paste-as-plain-text, which drops an
      // image, so the chord reads the clipboard itself.
      else if (action.kind === "paste") void pasteClipboard(instance.paneId, "terminal");
      // Clearing the selection once it is copied lets the next Ctrl+C
      // interrupt; a refused copy keeps it. Nothing selected copies nothing.
      else if (hasCopyText(instance)) {
        void copySelection(instance.paneId, "terminal").then((copied) => {
          if (!copied) return;
          setKept(instance, null);
          term.clearSelection();
        });
      }
    };
    // xterm sends a pending composition early for a key that reaches its own
    // handler, and a key taken here never does. macOS Chromium delivers the
    // Return that ends a Hangul syllable as a second keydown after
    // `compositionend`, so ⇧↩ right after "안녕" would reach the program as
    // "안", the newline, then "녕". The key waits one task instead, queued
    // behind xterm's send, so the syllable goes first; a key pressed while
    // one waits queues behind it rather than overtaking it.
    if (instance.commitPending || instance.keysWaiting > 0) {
      instance.keysWaiting += 1;
      setTimeout(() => {
        instance.keysWaiting -= 1;
        if (instances.get(instance.paneId) === instance) run();
      }, 0);
    } else run();
    return false;
  });
  // A selection drawn again, a drag or select-all, replaces the kept text.
  term.onSelectionChange(() => {
    if (term.hasSelection()) setKept(instance, null);
    else instance.follow = null;
  });
  term.onData((data) => send(new TextEncoder().encode(data)));
  const linkProvider = registerTerminalLinks(term, () => instance.paneId, link, () => instance.actions);
  const wheel = (event: WheelEvent) => onWheel(instance.paneId, instance, event);
  const down = (event: MouseEvent) => onMouseDown(instance, event);
  const up = (event: MouseEvent) => onMouseUp(instance.paneId, instance, event);
  const copy = (event: ClipboardEvent) => onCopy(instance, event);
  element.addEventListener("wheel", wheel, { capture: true, passive: false });
  // The press and release are read on the way down and left to xterm, whose
  // selection needs them. Its own mouse reports never fire: Herdr's frames
  // carry no mouse mode (`selection.ts` on what they carry), so the program
  // hears about a click only through the core's terminal_click route.
  element.addEventListener("mousedown", down, { capture: true });
  element.addEventListener("mouseup", up, { capture: true });
  element.addEventListener("copy", copy, { capture: true });
  instance.disposeHandlers = () => {
    element.removeEventListener("wheel", wheel, { capture: true });
    element.removeEventListener("mousedown", down, { capture: true });
    element.removeEventListener("mouseup", up, { capture: true });
    element.removeEventListener("copy", copy, { capture: true });
    if (instance.wheel.frame !== null) cancelAnimationFrame(instance.wheel.frame);
    linkProvider.dispose();
  };
  followFont(instance);
  return instance;
}

/**
 * Shows the pane's terminal inside `host`, creating it on first sight, and
 * returns the function that parks it again. The host's size is watched, so a
 * grid change from Herdr's new geometry reaches the core as one
 * `terminal_resize` per pane (PRD S2 B6).
 */
export function attachTerminal(
  paneId: string,
  host: HTMLElement,
  dispatch: DispatchFn,
  actions: TerminalActions,
  scale: number,
): () => void {
  reconcileTerminalIds();
  let instance = instances.get(paneId);
  const fresh = !instance;
  if (!instance) {
    instance = createInstance(paneId, dispatch, actions, scale);
    instances.set(paneId, instance);
  }
  if (instance.host) throw new Error(`pane ${paneId} is already shown`);
  const shown = instance;
  host.append(shown.element);
  shown.host = host;
  shown.dispatch = dispatch;
  shown.actions = actions;
  if (fresh) {
    shown.term.open(shown.element);
    // Added after xterm's own listener, so this clearing task runs right
    // after xterm's send of the composed text (`commitPending`).
    shown.term.textarea?.addEventListener("compositionend", () => {
      shown.commitPending = true;
      setTimeout(() => {
        shown.commitPending = false;
      }, 0);
    });
    shown.renderer = loadTerminalRenderer(shown.term, (reason) => {
      if (probeEnabled()) useShellStore.getState().noteDiagnostic(`terminal.renderer pane=${paneId} mode=dom reason=${reason}`);
    });
    // Clicking into a pane is the operator moving keyboard focus; the core
    // owns the focus pane, so the click is an event and the header follows
    // the snapshot, not the click.
    shown.term.textarea?.addEventListener("focus", () => {
      holdKept(shown);
      noteTerminalFocus(shown.paneId);
      if (followingSnapshot) return;
      // A snapshot can lag a newer intent. Core owns focus idempotence and
      // coalescing, so every operator focus must reach it.
      // A remote pane's focus is its host's, so it goes there by the pane's
      // own scoped id; this machine's `focus_pane` never carries it.
      const targetId = remoteTargetOfPane(useShellStore.getState().rest, shown.paneId);
      shown.dispatch(
        targetId
          ? remoteControl(targetId, { action: "focus_pane", pane_id: shown.paneId })
          : { schema_version: 2, kind: "focus_pane", payload: { pane_id: shown.paneId, origin: "operator" } },
      );
    });
  }
  // The observer also fires once when it starts watching, with the grid the
  // attach below just sent; only a grid the host's new size changes goes out.
  const observer = new ResizeObserver(() => sendGrid(shown.paneId, shown, false, true));
  observer.observe(host);
  shown.observer = observer;
  const needsFrame = fresh || shown.stale;
  shown.stale = false;
  sendGrid(paneId, shown, needsFrame);
  if (!fresh) shown.term.refresh(0, shown.term.rows - 1);
  return () => {
    reconcileTerminalIds();
    observer.disconnect();
    shown.observer = null;
    shown.host = null;
    // Text kept off screen is the shown pane's; coming back to it later must
    // not find Ctrl+C taken for a copy nobody can see.
    shown.follow = null;
    setKept(shown, null);
    parkingLot().append(shown.element);
    // A pane closed while shown left the core's list before its view
    // unmounted, so the park decides for itself instead of waiting for a
    // later change to sweep it.
    if (!attachedPaneIds().includes(shown.paneId)) disposeInstance(shown.paneId, shown);
  };
}

/**
 * Disposes every parked instance whose pane the core no longer streams:
 * released beyond the attach window, or gone with its tab. A shown pane is
 * skipped here; its park checks the same list when the view unmounts.
 */
export function retainTerminals() {
  reconcileTerminalIds();
  const attached = new Set(attachedPaneIds());
  for (const [paneId, instance] of instances) {
    if (attached.has(paneId) || instance.host) continue;
    disposeInstance(paneId, instance);
  }
}
