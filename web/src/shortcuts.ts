// The shortcut registry: one table, command -> key per host (PRD S2 D-01).
//
// The browser column is what Chrome lets a page claim. Five chords Chrome keeps
// for itself (⌘T ⌘W ⌘⇧T ⌃Tab ⌃⇧Tab) moved to the ⌥ family, and ⌘⇧W
// (Chrome's close-window) moved with them (D-06); `moved` marks each so the
// sheet can say so. ⌘⇧N, Add project, has no browser chord: adding a project
// needs the desktop app's folder picker, which a browser tab does not have. The Electron column is the macOS chord set: the desktop host
// has no browser keeping chords, so the moved ones return to their native
// keys there. The one exception is ⌘E, which shows the Explorer in both
// hosts (issue 170) while the macOS set keeps it on the sidebar switch; the
// switch has no default chord here and can be bound in Settings. Its app
// menu is built from this same column (`desktop/src/main/menu.ts`), so the
// menu and the sheet cannot disagree.
//
// Chords match on `KeyboardEvent.code`, not `key`: macOS turns ⌥-letters into
// dead keys and symbols (⌥T is "†"), and a physical position is what the
// operator's fingers know.
//
// The numbered commands (PRD electron-digit-shortcuts-hints D-02) are the
// desktop app's ⌥1-9 (the nth row the sidebar's Agents list draws) and ⌘1-9
// (the nth tile of the device rail). Chrome keeps ⌘1-9 for its own tabs
// and a page cannot claim ⌥-digits reliably either, so the browser column has
// none: the sheet says so and nothing is intercepted there. The number itself
// is the screen order at the moment of the press, assigned by the web shell
// (`numbering.ts`), and the hold hint (`hints.ts`) shows that order.
//
// The table is written in macOS chords, and one rule gives every other
// system its keys (`modChord`): ⌘ is Ctrl+Shift on Windows and Linux, as in
// Windows Terminal, GNOME Terminal, WezTerm and kitty, and ⇧⌘ or ⌥⌘ is
// Alt+Shift, the layer Windows Terminal puts its panes on, so a plain Ctrl key
// (Ctrl+C, Ctrl+R, Ctrl+W, Ctrl+K) always reaches the shell and the agent
// CLIs. `PC_KEYS` holds the few commands that keep a different chord there,
// each with its reason; docs/UI_BEHAVIOR.md (Keyboard shortcuts per system)
// lists the whole table on both systems.

import type { HostKind } from "./host";
import { TOOLS, type Tool } from "./laneTools";

export type Chord = {
  code: string;
  meta?: boolean;
  alt?: boolean;
  shift?: boolean;
  ctrl?: boolean;
};

/**
 * Which keyboard convention the operator's machine follows: macOS, whose app
 * chords hold ⌘, or Windows and Linux, which have no ⌘ and whose terminal
 * apps put app chords on Ctrl+Shift.
 */
export type KeySystem = "mac" | "pc";

/** The system a platform name stands for: Node's `darwin`, the browser's `MacIntel` or `macOS`, or a user agent. */
export function keySystemOf(platform: string): KeySystem {
  return /mac|darwin|iphone|ipad/i.test(platform) ? "mac" : "pc";
}

/**
 * A macOS chord as `system` presses it: the one rule. ⌘ is Ctrl+Shift off
 * macOS; a ⇧ or ⌥ the macOS chord adds to ⌘ makes it Alt+Shift there, the
 * layer Windows Terminal puts its panes on (⌘T is Ctrl+Shift+T and ⇧⌘T
 * Alt+Shift+T), so no chord needs more than three keys and Ctrl alone never
 * leaves the shell. A chord without ⌘ (⌃Tab, ⌥1) is the same keys on every
 * system.
 */
export function modChord(chord: Chord, system: KeySystem): Chord {
  if (system === "mac" || !chord.meta) return chord;
  return chord.shift || chord.alt ? { code: chord.code, alt: true, shift: true } : { code: chord.code, ctrl: true, shift: true };
}

/**
 * The macOS chord a press on `system` stands for, which is how a chord is
 * stored, so one stored set reads the same on every system; null for a
 * press holding the Windows or Super key, which the system keeps. Alt+Shift
 * is stored as ⇧⌘, the rule's chord on that layer, although ⌥⌘ and ⌥⇧ are
 * the same keys there too.
 */
export function macChord(chord: Chord, system: KeySystem): Chord | null {
  if (system === "mac") return chord;
  if (chord.meta) return null;
  if (chord.ctrl && chord.shift && !chord.alt) return { code: chord.code, meta: true };
  if (chord.alt && chord.shift && !chord.ctrl) return { code: chord.code, meta: true, shift: true };
  return chord;
}

/** Copy and paste in a terminal, as macOS chords: ⌘C and ⌘V, which the rule makes Ctrl+Shift+C and Ctrl+Shift+V elsewhere. */
export const TERMINAL_COPY: Chord = { code: "KeyC", meta: true };
export const TERMINAL_PASTE: Chord = { code: "KeyV", meta: true };

/**
 * A chord inside a text field, a palette, a board or a document, where no
 * terminal holds the keyboard: the system's own command key, ⌘ on macOS and
 * Ctrl elsewhere, as browsers and editors use it (⌘↵ is Ctrl+Enter there).
 */
export function fieldChord(code: string, system: KeySystem): Chord {
  return system === "mac" ? { code, meta: true } : { code, ctrl: true };
}

/** Whether a press or a click holds the system's own command key (`fieldChord`). */
export function holdsFieldModifier(event: { metaKey: boolean; ctrlKey: boolean }, system: KeySystem): boolean {
  return system === "mac" ? event.metaKey : event.ctrlKey;
}

export type CommandId =
  | "new_tab"
  | "close_tab"
  | "reopen_closed_tab"
  | "new_workspace"
  | "start_agent"
  | "recent_area_tab"
  | "previous_recent_area_tab"
  | "recent_panel"
  | "previous_recent_panel"
  | "recent_project"
  | "previous_recent_project"
  | "search"
  | "open_file"
  | "toggle_left_sidebar"
  | "overview"
  | "sidebar_projects"
  | "add_backlog"
  | "toggle_device_rail"
  | "toggle_lane"
  | "toggle_sidebar_lens"
  | LaneToolCommandId
  | "toggle_lane_documents"
  | "keep_open"
  | "find_in_pane"
  | "save_file"
  | "move_to_trash"
  | "copy_path"
  | "copy_relative_path"
  | "explorer_cut"
  | "explorer_copy"
  | "explorer_paste"
  | "split_right"
  | "split_down"
  | "toggle_zoom"
  | "close_pane"
  | "text_larger"
  | "text_smaller"
  | "text_reset"
  | AreaCommandId
  | "settings"
  | "shortcuts"
  | NumberedCommandId;

/**
 * Moving between the Agent areas or the View areas and resizing the one in
 * use (PRD cmdk-navigation D-05): commands ⌘K used to carry, with no default
 * chord, bindable in Settings and in the desktop menu.
 */
export const AREA_COMMANDS = [
  "focus_next_agent_area",
  "focus_previous_agent_area",
  "grow_agent_area",
  "shrink_agent_area",
  "focus_next_view_area",
  "focus_previous_view_area",
  "grow_view_area",
  "shrink_view_area",
] as const;
export type AreaCommandId = (typeof AREA_COMMANDS)[number];

/**
 * One command per rail icon (PRD lane-last-tool-chords; PRD sidebar-task-lens
 * D-19, D-29): the lane opens on that tool, or closes when it already shows
 * it, as pressing the icon does. Each tool's chord is a letter of its name
 * (⇧⌘L Memory, ⇧⌘R PRs, ⇧⌘E Explorer, ⇧⌘G Changes), Alt+Shift with the
 * same letter elsewhere (PRD lane-two-columns D-08). In a browser tab Chrome
 * keeps ⇧⌘R (reload), so PRs moves to ⌥⇧R.
 */
export type LaneToolCommandId = `toggle_lane_${Tool}`;
export const LANE_TOOL_COMMANDS: readonly LaneToolCommandId[] = TOOLS.map((tool): LaneToolCommandId => `toggle_lane_${tool}`);
const LANE_TOOL_TITLES: Record<LaneToolCommandId, string> = {
  toggle_lane_memory: "Toggle Memory tool",
  toggle_lane_prs: "Toggle PRs tool",
  toggle_lane_explorer: "Toggle Explorer tool",
  toggle_lane_changes: "Toggle Changes tool",
};
const LANE_TOOL_CHORDS: Record<LaneToolCommandId, { browser: Chord | null; electron: Chord | null; moved: boolean }> = {
  toggle_lane_memory: { browser: { code: "KeyL", meta: true, shift: true }, electron: { code: "KeyL", meta: true, shift: true }, moved: false },
  toggle_lane_prs: { browser: { code: "KeyR", alt: true, shift: true }, electron: { code: "KeyR", meta: true, shift: true }, moved: true },
  toggle_lane_explorer: { browser: { code: "KeyE", meta: true, shift: true }, electron: { code: "KeyE", meta: true, shift: true }, moved: false },
  toggle_lane_changes: { browser: { code: "KeyG", meta: true, shift: true }, electron: { code: "KeyG", meta: true, shift: true }, moved: false },
};

export type Digit = 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9;
export const DIGITS: readonly Digit[] = [1, 2, 3, 4, 5, 6, 7, 8, 9];

/** What a numbered chord selects: a row of the Agents list, or a tile of the device rail. */
export type NumberedFamily = "agents" | "devices";
export type NumberedCommandId = `select_agent_${Digit}` | `select_device_${Digit}`;

type NumberedFamilySpec = {
  family: NumberedFamily;
  prefix: "select_agent" | "select_device";
  title: string;
  group: Command["group"];
  /** The modifiers every chord of the family shares on the desktop host; the hold hint reveals the family on exactly these. */
  modifiers: Omit<Chord, "code">;
};

// ⌘ is the device's and ⌥ the Agents list's. A tab has no number: ⌃ would
// take control bytes from the terminal, and ⌃⇥ already steps through tabs.
export const NUMBERED_FAMILIES: readonly NumberedFamilySpec[] = [
  { family: "agents", prefix: "select_agent", title: "Select agent", group: "Navigate", modifiers: { alt: true } },
  { family: "devices", prefix: "select_device", title: "Select device", group: "Navigate", modifiers: { meta: true } },
];

function numberedEntries(spec: NumberedFamilySpec): Command[] {
  return DIGITS.map((digit) => ({
    id: `${spec.prefix}_${digit}` as NumberedCommandId,
    title: `${spec.title} ${digit}`,
    group: spec.group,
    browser: null,
    electron: { code: `Digit${digit}`, ...spec.modifiers },
    moved: false,
  }));
}

/** The family and number a command id names, or null for every other command. */
export function numberedCommand(id: CommandId): { family: NumberedFamily; number: Digit } | null {
  const match = /^(select_agent|select_device)_([1-9])$/.exec(id);
  if (!match) return null;
  const spec = NUMBERED_FAMILIES.find((row) => row.prefix === match[1]);
  return spec ? { family: spec.family, number: Number(match[2]) as Digit } : null;
}

export function isNumberedCommand(id: CommandId): id is NumberedCommandId {
  return numberedCommand(id) !== null;
}

export type Command = {
  id: CommandId;
  title: string;
  group: "Tabs" | "Navigate" | "Panels" | "Panes" | "Help";
  browser: Chord | null;
  /** The Electron host's chord: the macOS chord, or the browser one where the macOS set has none. */
  electron: Chord | null;
  /** True when the browser chord differs from the desktop app's because Chrome keeps the desktop chord on this system. */
  moved: boolean;
  /** Not intercepted at the window: the chord reaches the terminal (⌘⌫ is ^U there) and its shell owner is a later stage. */
  passthrough?: Passthrough;
};

/** Why a chord is not intercepted; the sheet and Settings word each in the operator's language. */
export type Passthrough = "explorer" | "explorer_terminal_clears_line";

export const REGISTRY: readonly Command[] = [
  { id: "new_tab", title: "New tab", group: "Tabs", browser: { code: "KeyT", alt: true }, electron: { code: "KeyT", meta: true }, moved: true },
  { id: "close_tab", title: "Close focused view or pane", group: "Tabs", browser: { code: "KeyW", alt: true }, electron: { code: "KeyW", meta: true }, moved: true },
  { id: "reopen_closed_tab", title: "Reopen closed tab", group: "Tabs", browser: { code: "KeyT", alt: true, shift: true }, electron: { code: "KeyT", meta: true, shift: true }, moved: true },
  { id: "new_workspace", title: "Add project", group: "Navigate", browser: null, electron: { code: "KeyN", meta: true, shift: true }, moved: false },
  { id: "start_agent", title: "Start agent", group: "Navigate", browser: null, electron: { code: "KeyN", meta: true }, moved: false },
  { id: "recent_area_tab", title: "Next recent tab", group: "Navigate", browser: { code: "Backquote", alt: true }, electron: { code: "Tab", ctrl: true }, moved: true },
  { id: "previous_recent_area_tab", title: "Previous recent tab", group: "Navigate", browser: { code: "Backquote", alt: true, shift: true }, electron: { code: "Tab", ctrl: true, shift: true }, moved: true },
  { id: "recent_panel", title: "Next panel (all projects)", group: "Navigate", browser: null, electron: null, moved: false },
  { id: "previous_recent_panel", title: "Previous panel (all projects)", group: "Navigate", browser: null, electron: null, moved: false },
  { id: "recent_project", title: "Next recent project", group: "Navigate", browser: { code: "Tab", alt: true }, electron: { code: "Tab", alt: true }, moved: false },
  { id: "previous_recent_project", title: "Previous recent project", group: "Navigate", browser: { code: "Tab", alt: true, shift: true }, electron: { code: "Tab", alt: true, shift: true }, moved: false },
  ...numberedEntries(NUMBERED_FAMILIES[0]!),
  ...numberedEntries(NUMBERED_FAMILIES[1]!),
  { id: "search", title: "Search", group: "Navigate", browser: { code: "KeyK", meta: true }, electron: { code: "KeyK", meta: true }, moved: false },
  { id: "open_file", title: "Open file", group: "Navigate", browser: { code: "KeyP", meta: true }, electron: { code: "KeyP", meta: true }, moved: false },
  { id: "toggle_left_sidebar", title: "Toggle left sidebar", group: "Panels", browser: { code: "KeyB", meta: true }, electron: { code: "KeyB", meta: true }, moved: false },
  // Chrome keeps ⇧⌘B (bookmarks bar), so a browser tab turns the lens with ⌥⇧B (PRD sidebar-task-lens D-32).
  { id: "toggle_sidebar_lens", title: "Switch Projects and Tasks", group: "Panels", browser: { code: "KeyB", alt: true, shift: true }, electron: { code: "KeyB", meta: true, shift: true }, moved: true },
  { id: "overview", title: "Overview", group: "Navigate", browser: { code: "KeyO", meta: true, shift: true }, electron: { code: "KeyO", meta: true, shift: true }, moved: false },
  // Chrome keeps ⌘⇧M for the profile menu, so the browser form is ⌥⇧M (PRD overview-device-tasks B37).
  { id: "add_backlog", title: "Add to backlog", group: "Navigate", browser: { code: "KeyM", alt: true, shift: true }, electron: { code: "KeyM", meta: true, shift: true }, moved: true },
  { id: "sidebar_projects", title: "Focus sidebar", group: "Panels", browser: { code: "KeyP", meta: true, shift: true }, electron: { code: "KeyP", meta: true, shift: true }, moved: false },
  { id: "toggle_device_rail", title: "Toggle device rail", group: "Panels", browser: null, electron: null, moved: false },
  { id: "toggle_lane", title: "Toggle right lane", group: "Panels", browser: { code: "KeyE", meta: true }, electron: { code: "KeyE", meta: true }, moved: false },
  ...LANE_TOOL_COMMANDS.map((id): Command => ({ id, title: LANE_TOOL_TITLES[id], group: "Panels", ...LANE_TOOL_CHORDS[id] })),
  // The documents button has no chord until one is bound: an open brings the documents in front.
  { id: "toggle_lane_documents", title: "Toggle documents", group: "Panels", browser: null, electron: null, moved: false },
  { id: "find_in_pane", title: "Find in pane", group: "Panes", browser: { code: "KeyF", meta: true }, electron: { code: "KeyF", meta: true }, moved: false },
  { id: "save_file", title: "Save file", group: "Panes", browser: { code: "KeyS", meta: true }, electron: { code: "KeyS", meta: true }, moved: false },
  { id: "keep_open", title: "Keep open", group: "Panes", browser: { code: "KeyK", meta: true, shift: true }, electron: { code: "KeyK", meta: true, shift: true }, moved: false },
  { id: "split_right", title: "Split right", group: "Panes", browser: { code: "KeyD", meta: true }, electron: { code: "KeyD", meta: true }, moved: false },
  { id: "split_down", title: "Split down", group: "Panes", browser: { code: "KeyD", meta: true, shift: true }, electron: { code: "KeyD", meta: true, shift: true }, moved: false },
  { id: "toggle_zoom", title: "Zoom pane", group: "Panes", browser: { code: "Enter", meta: true, shift: true }, electron: { code: "Enter", meta: true, shift: true }, moved: false },
  { id: "close_pane", title: "Close pane", group: "Panes", browser: { code: "KeyW", alt: true, shift: true }, electron: { code: "KeyW", meta: true, shift: true }, moved: true },
  { id: "text_larger", title: "Larger text", group: "Panes", browser: { code: "Equal", meta: true }, electron: { code: "Equal", meta: true }, moved: false },
  { id: "text_smaller", title: "Smaller text", group: "Panes", browser: { code: "Minus", meta: true }, electron: { code: "Minus", meta: true }, moved: false },
  { id: "text_reset", title: "Reset text size", group: "Panes", browser: { code: "Digit0", meta: true }, electron: { code: "Digit0", meta: true }, moved: false },
  { id: "focus_next_agent_area", title: "Focus next Agent area", group: "Panes", browser: null, electron: null, moved: false },
  { id: "focus_previous_agent_area", title: "Focus previous Agent area", group: "Panes", browser: null, electron: null, moved: false },
  { id: "grow_agent_area", title: "Grow Agent area", group: "Panes", browser: null, electron: null, moved: false },
  { id: "shrink_agent_area", title: "Shrink Agent area", group: "Panes", browser: null, electron: null, moved: false },
  { id: "focus_next_view_area", title: "Focus next View area", group: "Panes", browser: null, electron: null, moved: false },
  { id: "focus_previous_view_area", title: "Focus previous View area", group: "Panes", browser: null, electron: null, moved: false },
  { id: "grow_view_area", title: "Grow View area", group: "Panes", browser: null, electron: null, moved: false },
  { id: "shrink_view_area", title: "Shrink View area", group: "Panes", browser: null, electron: null, moved: false },
  { id: "move_to_trash", title: "Move to Trash", group: "Panes", browser: { code: "Backspace", meta: true }, electron: { code: "Backspace", meta: true }, moved: false, passthrough: "explorer_terminal_clears_line" },
  // The Explorer's two path copies, VS Code's chords. Chrome keeps ⌥⌘C (Inspect), so a browser tab has no chord; the tree answers them, as it does Move to Trash, so a terminal keeps its keys.
  { id: "copy_path", title: "Copy Path", group: "Panes", browser: null, electron: { code: "KeyC", meta: true, alt: true }, moved: false, passthrough: "explorer" },
  { id: "copy_relative_path", title: "Copy Relative Path", group: "Panes", browser: null, electron: { code: "KeyC", meta: true, alt: true, shift: true }, moved: false, passthrough: "explorer" },
  // The tree's clipboard, the system's own chords: the tree answers them while it holds the keyboard, so a terminal keeps ⌘C (copy a selection) and ⌘V (paste). Fixed, as the text fields' chords are.
  { id: "explorer_cut", title: "Cut File or Folder", group: "Panes", browser: { code: "KeyX", meta: true }, electron: { code: "KeyX", meta: true }, moved: false, passthrough: "explorer" },
  { id: "explorer_copy", title: "Copy File or Folder", group: "Panes", browser: { code: "KeyC", meta: true }, electron: { code: "KeyC", meta: true }, moved: false, passthrough: "explorer" },
  { id: "explorer_paste", title: "Paste File or Folder", group: "Panes", browser: { code: "KeyV", meta: true }, electron: { code: "KeyV", meta: true }, moved: false, passthrough: "explorer" },
  { id: "settings", title: "Settings", group: "Help", browser: { code: "Comma", alt: true }, electron: { code: "Comma", meta: true }, moved: true },
  { id: "shortcuts", title: "Keyboard shortcuts", group: "Help", browser: { code: "Slash", meta: true }, electron: { code: "Slash", meta: true }, moved: false },
];

/**
 * The commands whose chord on Windows and Linux is not the rule's
 * (`modChord`): a dominant convention there, or a chord the system keeps.
 * The same keys hold in both hosts; a host without the command stays
 * without it, and an operator's own binding replaces the entry.
 */
export const PC_KEYS: Readonly<Partial<Record<CommandId, { chord?: Chord; browser?: Chord; passthrough?: Passthrough }>>> = {
  // Alt+Tab is the system's window switcher and never reaches an app, so the
  // cycle moves to ` under the rule (macOS keeps ⌘` for its own window cycle,
  // which is why it uses ⌥Tab). Going back adds Alt to the Ctrl+Shift the
  // cycle holds: a held cycle commits when its Ctrl is released, so the
  // rule's Alt+Shift+` could not step back inside it, and GNOME keeps
  // Alt+Shift+` (switch-group-backward) for itself.
  recent_project: { chord: { code: "Backquote", ctrl: true, shift: true } },
  previous_recent_project: { chord: { code: "Backquote", ctrl: true, alt: true, shift: true } },
  // Chrome keeps Alt+Shift+T (focus the toolbar) on Windows and Linux, and the
  // macOS browser chord ⌥⇧T is those same keys there, so a browser tab
  // reopens with the one free chord on that key.
  reopen_closed_tab: { browser: { code: "KeyT", ctrl: true, alt: true, shift: true } },
  // Chrome keeps Alt+Shift+B (focus the bookmarks bar) on Windows and Linux,
  // and the macOS browser chord ⌥⇧B is those same keys there, so a browser
  // tab turns the sidebar's list with the one free chord on that key.
  toggle_sidebar_lens: { browser: { code: "KeyB", ctrl: true, alt: true, shift: true } },
  // Windows Terminal, GNOME Terminal, WezTerm, VS Code and every browser size
  // text with Ctrl and =, - or 0.
  text_larger: { chord: { code: "Equal", ctrl: true } },
  text_smaller: { chord: { code: "Minus", ctrl: true } },
  text_reset: { chord: { code: "Digit0", ctrl: true } },
  // Windows Explorer and the Linux file managers move a file to the Recycle
  // Bin or Trash with Delete.
  move_to_trash: { chord: { code: "Delete" }, passthrough: "explorer" },
  // The rule would give Copy Relative Path the same Alt+Shift+C as Copy Path,
  // VS Code's own chord for it is a two-step sequence this registry cannot
  // hold, and Alt+Shift+R opens the PRs tool (PRD sidebar-task-lens B25), so
  // it takes the free Alt+Shift+H.
  copy_relative_path: { chord: { code: "KeyH", alt: true, shift: true }, passthrough: "explorer" },
  // A terminal copies with Ctrl+Shift+C there, so Ctrl+C, X and V are free for the tree, as in every file manager.
  explorer_cut: { chord: { code: "KeyX", ctrl: true }, passthrough: "explorer" },
  explorer_copy: { chord: { code: "KeyC", ctrl: true }, passthrough: "explorer" },
  explorer_paste: { chord: { code: "KeyV", ctrl: true }, passthrough: "explorer" },
};

/** Chords Chrome or the system keeps for itself, per system; a browser chord using one is a registry error. */
const CHROME_RESERVED: Readonly<Record<KeySystem, readonly Chord[]>> = {
  mac: [
    { code: "Tab", meta: true },
    { code: "KeyT", meta: true },
    { code: "KeyW", meta: true },
    { code: "KeyT", meta: true, shift: true },
    { code: "KeyN", meta: true, shift: true },
    { code: "KeyW", meta: true, shift: true },
    { code: "Tab", ctrl: true },
    { code: "Tab", ctrl: true, shift: true },
    { code: "KeyN", meta: true },
    { code: "KeyQ", meta: true },
    { code: "KeyH", meta: true },
    { code: "KeyM", meta: true },
    // Chrome's profile switcher.
    { code: "KeyM", meta: true, shift: true },
    { code: "Comma", meta: true },
    // Hard reload and the bookmarks bar.
    { code: "KeyR", meta: true, shift: true },
    { code: "KeyB", meta: true, shift: true },
  ],
  // Chrome's reserved commands on Windows and Linux (new and closed tabs and
  // windows, tab switching, quit), its Alt+Shift keys (focus the toolbar,
  // focus inactive dialogs, the feedback form, split view, the bookmarks
  // bar), which a page
  // could take but a keyboard user relies on, and the system's window
  // switcher.
  pc: [
    { code: "KeyT", ctrl: true },
    { code: "KeyW", ctrl: true },
    { code: "KeyN", ctrl: true },
    { code: "KeyT", ctrl: true, shift: true },
    { code: "KeyN", ctrl: true, shift: true },
    { code: "KeyW", ctrl: true, shift: true },
    { code: "KeyQ", ctrl: true, shift: true },
    { code: "Tab", ctrl: true },
    { code: "Tab", ctrl: true, shift: true },
    { code: "KeyT", alt: true, shift: true },
    { code: "KeyA", alt: true, shift: true },
    { code: "KeyI", alt: true, shift: true },
    { code: "KeyN", alt: true, shift: true },
    { code: "KeyB", alt: true, shift: true },
    { code: "Tab", alt: true },
    { code: "Tab", alt: true, shift: true },
  ],
};

export function chordEquals(a: Chord, b: Chord): boolean {
  return (
    a.code === b.code &&
    !!a.meta === !!b.meta &&
    !!a.alt === !!b.alt &&
    !!a.shift === !!b.shift &&
    !!a.ctrl === !!b.ctrl
  );
}

export function isChromeReserved(chord: Chord, system: KeySystem): boolean {
  return CHROME_RESERVED[system].some((reserved) => chordEquals(reserved, chord));
}

// A browser chord moved off the desktop one on macOS returns to the desktop
// chord on Windows and Linux unless Chrome keeps that chord there too, so the
// only browser chords that differ are the ones Chrome forces.
function pcCommand(command: Command): Command {
  const deviation = PC_KEYS[command.id];
  const keys = (chord: Chord | null) => chord && (deviation?.chord ?? modChord(chord, "pc"));
  const electron = keys(command.electron);
  const moved = command.moved && electron !== null && isChromeReserved(electron, "pc");
  return {
    ...command,
    browser: deviation?.browser ?? (command.moved && !moved ? electron : keys(command.browser)),
    electron,
    moved,
    ...(deviation?.passthrough ? { passthrough: deviation.passthrough } : {}),
  };
}

const PC_REGISTRY: readonly Command[] = REGISTRY.map(pcCommand);

/** The registry as `system` presses it: the macOS table itself, or every chord through the rule and `PC_KEYS`. */
export function systemRegistry(system: KeySystem): readonly Command[] {
  return system === "mac" ? REGISTRY : PC_REGISTRY;
}

type KeyEventLike = Pick<KeyboardEvent, "code" | "metaKey" | "altKey" | "shiftKey" | "ctrlKey">;

export function chordFromEvent(event: KeyEventLike): Chord {
  return { code: event.code, meta: event.metaKey, alt: event.altKey, shift: event.shiftKey, ctrl: event.ctrlKey };
}

/** A command's chord on `host`. */
export function hostChord(command: Command, host: HostKind): Chord | null {
  return host === "electron" ? command.electron : command.browser;
}

/** The command a keydown names on `host`, or null. */
export function matchHost(event: KeyEventLike, registry: readonly Command[], host: HostKind): Command | null {
  const chord = chordFromEvent(event);
  return (
    registry.find((command) => {
      const bound = hostChord(command, host);
      return bound && !command.passthrough && chordEquals(bound, chord);
    }) ?? null
  );
}

// The pane commands an operator may rebind (PRD S5 D-07): the eight macOS
// pane commands less Toggle Conversation, which the web shell has no surface
// for, plus the two sidebar switches (view and device rail) that have no macOS
// pane command. Each host keeps its own set in the core, because the hosts reserve
// different keys (Chrome keeps ⌘W): the browser's in
// `ui_state.browser_shortcut_bindings`, and the desktop app's in
// `ui_state.shortcut_bindings`, the macOS set in the removed native app's
// format and command names (user decision 2026-09-26: the desktop app
// honours the operator's existing shortcut settings).
export const EDITABLE_PANE_COMMANDS: readonly CommandId[] = [
  "recent_area_tab", "previous_recent_area_tab", "recent_panel", "previous_recent_panel",
  "overview", "sidebar_projects",
  "toggle_device_rail",
  "toggle_sidebar_lens",
  ...LANE_TOOL_COMMANDS,
  "split_right",
  "split_down",
  "toggle_zoom",
  "close_pane",
  "text_larger",
  "text_smaller",
  "text_reset",
  ...AREA_COMMANDS,
  "copy_path",
  "copy_relative_path",
];

/**
 * The macOS set's name for each editable command. `toggle_device_rail`
 * has no pane command there: the set keeps the key and the desktop host
 * ignores it, the way this shell ignores
 * `toggle_conversation`.
 */
const MACOS_KEYS: Readonly<Partial<Record<CommandId, string>>> = {
  overview: "overview",
  sidebar_projects: "sidebar_projects",
  toggle_device_rail: "toggle_device_rail",
  split_right: "split_right",
  split_down: "split_down",
  toggle_zoom: "toggle_zoom",
  close_pane: "close_pane",
  text_larger: "increase_text_size",
  text_smaller: "decrease_text_size",
  text_reset: "reset_text_size",
};

/** macOS pane commands the web shell has no surface for: kept in the set, never run here. */
const MACOS_ONLY_KEYS: readonly string[] = ["toggle_conversation"];

// A physical key a chord may use. Anything else (a dead key, an IME process
// key, a lone modifier) is not a chord this registry can match reliably.
const BINDABLE_CODE = /^(Key[A-Z]|Digit[0-9]|Enter|Tab|Backquote|Minus|Equal|BracketLeft|BracketRight|Backslash|Semicolon|Quote|Comma|Period|Slash|Arrow(Up|Down|Left|Right))$/;

/** One stored browser chord: its modifiers in a fixed order, then the physical key. */
export function serializeChord(chord: Chord): string {
  return [chord.ctrl && "ctrl", chord.alt && "alt", chord.shift && "shift", chord.meta && "meta", chord.code].filter(Boolean).join("+");
}

export function parseChord(text: string): Chord | null {
  const parts = text.split("+");
  const code = parts.pop() ?? "";
  if (!BINDABLE_CODE.test(code)) return null;
  const chord: Chord = { code };
  for (const part of parts) {
    if (part !== "ctrl" && part !== "alt" && part !== "shift" && part !== "meta") return null;
    const key = part === "ctrl" ? "ctrl" : part === "alt" ? "alt" : part === "shift" ? "shift" : "meta";
    if (chord[key]) return null;
    chord[key] = true;
  }
  return chord;
}

// The macOS set's keys: one printable ASCII key, named by the character it
// types unshifted, or Return.
const MACOS_KEY_CODES: Readonly<Record<string, string>> = {
  tab: "Tab",
  return: "Enter",
  "`": "Backquote",
  "-": "Minus",
  "=": "Equal",
  "[": "BracketLeft",
  "]": "BracketRight",
  "\\": "Backslash",
  ";": "Semicolon",
  "'": "Quote",
  ",": "Comma",
  ".": "Period",
  "/": "Slash",
};

function macosKeyCode(key: string): string | null {
  if (/^[a-z]$/.test(key)) return `Key${key.toUpperCase()}`;
  if (/^[0-9]$/.test(key)) return `Digit${key}`;
  return MACOS_KEY_CODES[key] ?? null;
}

function macosKeyName(code: string): string | null {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3).toLowerCase();
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  return Object.entries(MACOS_KEY_CODES).find(([, known]) => known === code)?.[0] ?? null;
}

/** A macOS set chord ("command+shift+return"), read leniently: case and order do not matter. */
export function parseMacosChord(text: string): Chord | null {
  const parts = text.toLowerCase().replace(/ /g, "").split("+");
  const key = parts.pop() ?? "";
  if (parts.length === 0) return null;
  const code = macosKeyCode(key === "enter" ? "return" : key);
  if (!code) return null;
  const chord: Chord = { code };
  for (const part of parts) {
    const modifier =
      part === "cmd" || part === "command" ? "meta" : part === "ctrl" || part === "control" ? "ctrl" : part === "opt" || part === "option" || part === "alt" ? "alt" : part === "shift" ? "shift" : null;
    if (!modifier || chord[modifier]) return null;
    chord[modifier] = true;
  }
  return chord;
}

/** A chord in the macOS set's canonical form (modifiers command, control, option, shift), or null when its key has no name there. */
export function serializeMacosChord(chord: Chord): string | null {
  const key = macosKeyName(chord.code);
  if (!key) return null;
  return [chord.meta && "command", chord.ctrl && "control", chord.alt && "option", chord.shift && "shift", key].filter(Boolean).join("+");
}

/**
 * Chords macOS or the app keeps on the desktop host. ⌘1-9 are not here: the
 * registry's own numbered commands hold them, so a pane chord bound onto one
 * is refused as that command's, by name.
 */
const MACOS_RESERVED: readonly Chord[] = ["Tab", "KeyQ", "KeyH", "KeyM", "KeyS", "KeyW", "Comma"].map((code) => ({ code, meta: true }));

/** Chords Windows and Linux keep from the desktop app: the window switcher. */
const PC_RESERVED: readonly Chord[] = [{ code: "Tab", alt: true }, { code: "Tab", alt: true, shift: true }];

/** The key a command's chord is stored under in `host`'s set. */
export function storedKey(id: CommandId, host: HostKind): string {
  return host === "electron" ? (MACOS_KEYS[id] ?? id) : id;
}

/** A stored chord as `system` presses it; a set stores macOS chords (`macChord`). */
export function parseStoredChord(text: string, host: HostKind, system: KeySystem): Chord | null {
  const chord = host === "electron" ? parseMacosChord(text) : parseChord(text);
  return chord && modChord(chord, system);
}

/** How a press on `system` is stored, or null when the set cannot hold it. */
export function serializeStoredChord(chord: Chord, host: HostKind, system: KeySystem): string | null {
  const stored = macChord(chord, system);
  if (!stored) return null;
  return host === "electron" ? serializeMacosChord(stored) : serializeChord(stored);
}

/** The stored maps the snapshot's `ui_state` carries, one per host. */
export type StoredShortcutSets = {
  shortcut_bindings?: Record<string, string>;
  browser_shortcut_bindings?: Record<string, string>;
} | null | undefined;

/** `host`'s own stored set. */
export function storedBindings(uiState: StoredShortcutSets, host: HostKind): Record<string, string> | undefined {
  return host === "electron" ? uiState?.shortcut_bindings : uiState?.browser_shortcut_bindings;
}

function withChord(command: Command, chord: Chord | null, host: HostKind): Command {
  return host === "electron" ? { ...command, electron: chord } : { ...command, browser: chord, moved: false };
}

/**
 * Why a chord cannot be bound. `chord` is the offending chord as `system`
 * prints it; the operator's wording lives in `bindingProblemText`
 * (`shortcutLabels.ts`).
 */
export type BindingProblem =
  | { code: "system_key" | "native_key" | "browser_key" }
  | { code: "modifier"; modifier: "desktop_mac" | "desktop_pc" | "browser_mac" | "browser_pc" }
  | { code: "mac_reserved" | "system_reserved" | "terminal_copy" | "terminal_paste"; chord: string }
  | { code: "browser_reserved"; chord: string; macos: boolean }
  | { code: "conflict"; chord: string; commandId: CommandId };

/**
 * Why `chord` cannot become `id`'s binding on `host` in `registry`, or null
 * when it can. The same rules decide a stored set on load, so a chord the
 * editor refuses can never become effective by being written to the core
 * directly. `chord` and `registry` are the keys `system` presses; the
 * desktop host's rules are the macOS set's own, with ⌘ read as Ctrl+Shift
 * off macOS.
 */
export function bindingProblem(id: CommandId, chord: Chord, registry: readonly Command[], host: HostKind, system: KeySystem): BindingProblem | null {
  const shown = displayChord(chord, system);
  if (system === "pc" && chord.meta) return { code: "system_key" };
  if (host === "electron") {
    // Exactly one of Ctrl and Alt: Ctrl+Alt+Shift is the rule's image of no
    // macOS chord, so a set holding one would be refused on a Mac.
    const mod = system === "mac" ? !!chord.meta : !!chord.shift && !!chord.ctrl !== !!chord.alt;
    if (!macosKeyName(chord.code)) return { code: "native_key" };
    if (!mod && !(isCycleCommand(id) && (chord.ctrl || chord.alt))) return { code: "modifier", modifier: system === "mac" ? "desktop_mac" : "desktop_pc" };
    if (system === "mac" && MACOS_RESERVED.some((reserved) => chordEquals(reserved, chord))) return { code: "mac_reserved", chord: shown };
    if (system === "pc" && PC_RESERVED.some((reserved) => chordEquals(reserved, chord))) return { code: "system_reserved", chord: shown };
  } else {
    if (!BINDABLE_CODE.test(chord.code)) return { code: "browser_key" };
    if (!chord.meta && !chord.ctrl && !chord.alt) return { code: "modifier", modifier: system === "mac" ? "browser_mac" : "browser_pc" };
    if (isChromeReserved(chord, system)) return { code: "browser_reserved", chord: shown, macos: system === "mac" };
  }
  if (chordEquals(chord, modChord(TERMINAL_COPY, system))) return { code: "terminal_copy", chord: shown };
  if (chordEquals(chord, modChord(TERMINAL_PASTE, system))) return { code: "terminal_paste", chord: shown };
  const taken = registry.find((command) => {
    const bound = command.id === id ? null : hostChord(command, host);
    return bound && chordEquals(bound, chord);
  });
  if (taken) return { code: "conflict", chord: shown, commandId: taken.id };
  return null;
}

export type EffectiveRegistry = { registry: readonly Command[]; diagnostic: string | null };

/**
 * The registry `host` runs, with the operator's stored pane chords applied.
 * A stored set that names an unknown command, holds a chord this host cannot
 * use, or collides with another command is dropped as a whole and the
 * defaults run; the diagnostic
 * says why. Collisions are judged on the whole set, so two commands that
 * trade chords are one valid set.
 */
/** Stored commands that no longer exist: ignored with a diagnostic, never a reason to drop the rest of the set (`sidebar_agents`, PRD overview-device-tasks D-16; `toggle_file_tree` and `toggle_lane_files`, PRD lane-two-columns D-03, D-10). */
const RETIRED_KEYS: readonly string[] = ["toggle_sidebar_view", "project_home", "sidebar_agents", "toggle_lane_work", "toggle_file_tree", "toggle_lane_files"];

export function effectiveRegistry(stored: Record<string, string> | null | undefined, host: HostKind, system: KeySystem): EffectiveRegistry {
  const which = host === "electron" ? "pane" : "browser";
  const defaults = systemRegistry(system);
  const retired = Object.keys(stored ?? {}).some((key) => RETIRED_KEYS.includes(key));
  const diagnostic = retired ? "Stored project_home, toggle_sidebar_view, sidebar_agents, toggle_lane_work, toggle_file_tree or toggle_lane_files shortcut is retired and was ignored." : null;
  const entries = Object.entries(stored ?? {}).filter(([key]) => !RETIRED_KEYS.includes(key) && !(host === "electron" && MACOS_ONLY_KEYS.includes(key)));
  if (entries.length === 0) return { registry: defaults, diagnostic };
  let registry: Command[] = [...defaults];
  const applied: [CommandId, Chord][] = [];
  for (const [key, text] of entries) {
    const id = EDITABLE_PANE_COMMANDS.find((command) => storedKey(command, host) === key);
    if (!id) return { registry: defaults, diagnostic: `Stored ${which} shortcuts name an unknown command (${key}); defaults are in use.` };
    if (text === "none") {
      registry = registry.map((command) => command.id === id ? withChord(command, null, host) : command);
      continue;
    }
    const chord = parseStoredChord(text, host, system);
    if (!chord) return { registry: defaults, diagnostic: `Stored ${which} shortcut for ${key} was not usable (unreadable chord); defaults are in use.` };
    registry = registry.map((command) => (command.id === id ? withChord(command, chord, host) : command));
    applied.push([id, chord]);
  }
  for (const [id, chord] of applied) {
    const problem = bindingProblem(id, chord, registry, host, system);
    if (problem) return { registry: defaults, diagnostic: `Stored ${which} shortcut for ${storedKey(id, host)} was not usable (${problem.code}${problem.code === "conflict" ? ` ${problem.commandId}` : ""}); defaults are in use.` };
  }
  return { registry, diagnostic };
}

const resolved = new Map<string, { stored: Record<string, string> | null | undefined; value: EffectiveRegistry }>();

/**
 * `effectiveRegistry` for the stored set the snapshot carries now, computed
 * once per set: the store shares an unchanged section by reference, so a
 * keystroke reuses the last resolution instead of re-validating every chord.
 */
export function resolvedRegistry(stored: Record<string, string> | null | undefined, host: HostKind, system: KeySystem): EffectiveRegistry {
  const key = `${host}:${system}`;
  const last = resolved.get(key);
  if (last && last.stored === stored) return last.value;
  const value = effectiveRegistry(stored, host, system);
  resolved.set(key, { stored, value });
  return value;
}

/** The registry `host` runs on `system`: its column with its own stored set applied. */
export function hostRegistry(uiState: StoredShortcutSets, host: HostKind, system: KeySystem): EffectiveRegistry {
  return resolvedRegistry(storedBindings(uiState, host), host, system);
}

/** A command's default chord on `host` and `system`, before any stored set. */
export function defaultChord(id: CommandId, host: HostKind, system: KeySystem): Chord | null {
  const command = systemRegistry(system).find((row) => row.id === id);
  return command ? hostChord(command, host) : null;
}

const MAC_GLYPHS: Record<string, string> = {
  Comma: ",",
  Period: ".",
  Semicolon: ";",
  Quote: "'",
  BracketLeft: "[",
  BracketRight: "]",
  Backslash: "\\",
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
  Backquote: "`",
  Tab: "⇥",
  Enter: "↩",
  Equal: "=",
  Minus: "-",
  Slash: "/",
  Backspace: "⌫",
  Delete: "⌦",
};

// Windows and Linux name keys in words and join a chord with "+", modifiers
// first in Windows' order Ctrl, Alt, Shift, so a chord the rule makes begins
// with "Ctrl+Shift+" or "Alt+Shift+".
const PC_NAMES: Record<string, string> = {
  ...MAC_GLYPHS,
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  Tab: "Tab",
  Enter: "Enter",
  Backspace: "Backspace",
  Delete: "Delete",
};

function keyName(code: string, names: Record<string, string>): string {
  return names[code] ?? (code.startsWith("Key") ? code.slice(3) : code.startsWith("Digit") ? code.slice(5) : code);
}

/** The chord as the operator reads it on `system`: macOS glyphs in macOS modifier order ("⇧⌘T"), or words in Windows' order ("Alt+Shift+T"). */
export function displayChord(chord: Chord, system: KeySystem): string {
  if (system === "pc") {
    return [chord.ctrl && "Ctrl", chord.alt && "Alt", chord.shift && "Shift", chord.meta && "Win", keyName(chord.code, PC_NAMES)].filter(Boolean).join("+");
  }
  return `${chord.ctrl ? "⌃" : ""}${chord.alt ? "⌥" : ""}${chord.shift ? "⇧" : ""}${chord.meta ? "⌘" : ""}${keyName(chord.code, MAC_GLYPHS)}`;
}

/** The modifiers alone as `system` presses them ("⌘", "Ctrl+Shift"): what a hold hint asks the operator to hold. */
export function modifierLabel(modifiers: Omit<Chord, "code">, system: KeySystem): string {
  if (system === "pc") {
    return [modifiers.ctrl && "Ctrl", modifiers.alt && "Alt", modifiers.shift && "Shift", modifiers.meta && "Win"].filter(Boolean).join("+");
  }
  return `${modifiers.ctrl ? "⌃" : ""}${modifiers.alt ? "⌥" : ""}${modifiers.shift ? "⇧" : ""}${modifiers.meta ? "⌘" : ""}`;
}

/** A command's chord on `host` in `registry` (the keys `system` presses) as the operator reads it, or "" when it has none. */
export function displayCommand(id: CommandId, host: HostKind, registry: readonly Command[], system: KeySystem): string {
  const command = registry.find((row) => row.id === id);
  const chord = command ? hostChord(command, host) : null;
  return chord ? displayChord(chord, system) : "";
}

/**
 * A numbered family's chords on `host` in number order, or null where the
 * host has none: the hold hint reveals a family when the held modifiers are
 * exactly its chords' (PRD electron-digit-shortcuts-hints D-03, D-04).
 */
export function familyChords(family: NumberedFamily, registry: readonly Command[], host: HostKind): Chord[] | null {
  const spec = NUMBERED_FAMILIES.find((row) => row.family === family);
  if (!spec) return null;
  const chords = DIGITS.map((digit) => {
    const command = registry.find((row) => row.id === `${spec.prefix}_${digit}`);
    return command ? hostChord(command, host) : null;
  });
  return chords.every((chord): chord is Chord => chord !== null) ? chords : null;
}

/** The modifiers a family's chords share on `host`, as a chord without a key, or null where the host has none. */
export function familyModifiers(family: NumberedFamily, registry: readonly Command[], host: HostKind): Omit<Chord, "code"> | null {
  const chords = familyChords(family, registry, host);
  if (!chords) return null;
  const [first] = chords;
  const modifiers = { meta: !!first!.meta, alt: !!first!.alt, shift: !!first!.shift, ctrl: !!first!.ctrl };
  const shared = chords.every((chord) => !!chord.meta === modifiers.meta && !!chord.alt === modifiers.alt && !!chord.shift === modifiers.shift && !!chord.ctrl === modifiers.ctrl);
  return shared ? modifiers : null;
}

/** One line of the shortcut sheet: a command, or a numbered family folded into one row with its range. */
export type SheetRow = { id: CommandId; title: string; chord: string | null; moved: boolean; movedFrom?: string; passthrough?: Passthrough; range?: string };

/**
 * The sheet's rows for `group` on `host`, in registry order, with each
 * numbered family folded into one row ("Select device 1-9", "⌘1 … ⌘9"), or a
 * null chord where the host has none (B3, B4). A moved browser row names the
 * desktop chord Chrome keeps.
 */
export function sheetRows(group: Command["group"], registry: readonly Command[], host: HostKind, system: KeySystem): SheetRow[] {
  const rows: SheetRow[] = [];
  const folded = new Set<NumberedFamily>();
  for (const command of registry) {
    if (command.group !== group) continue;
    const numbered = numberedCommand(command.id);
    if (!numbered) {
      const chord = hostChord(command, host);
      const movedFrom = command.moved && command.electron ? displayChord(command.electron, system) : undefined;
      rows.push({ id: command.id, title: command.title, chord: chord ? displayChord(chord, system) : null, moved: command.moved, movedFrom, passthrough: command.passthrough });
      continue;
    }
    if (folded.has(numbered.family)) continue;
    folded.add(numbered.family);
    const spec = NUMBERED_FAMILIES.find((row) => row.family === numbered.family)!;
    const chords = familyChords(numbered.family, registry, host);
    rows.push({
      id: command.id,
      title: `${spec.title} ${DIGITS[0]}-${DIGITS[DIGITS.length - 1]}`,
      range: `${DIGITS[0]}-${DIGITS[DIGITS.length - 1]}`,
      chord: chords ? `${displayChord(chords[0]!, system)} … ${displayChord(chords[chords.length - 1]!, system)}` : null,
      moved: false,
    });
  }
  return rows;
}

/** Held navigation commands bypass native menu accelerators and share release routing. */
export function isCycleCommand(id: CommandId): boolean {
  return ["recent_area_tab", "previous_recent_area_tab", "recent_panel", "previous_recent_panel", "recent_project", "previous_recent_project"].includes(id);
}
export function releaseModifier(chord: Chord): "Control" | "Alt" | "Meta" | null {
  return chord.ctrl ? "Control" : chord.alt ? "Alt" : chord.meta ? "Meta" : null;
}
