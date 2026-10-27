// The app menu. Its commands and chords come from the web shell's registry
// (`web/src/shortcuts.ts`, Electron column), so the menu, the ⌘/ sheet and
// the window listener read one table (desktop PRD D-04). A click sends the
// command id to the shell over the bridge; a chord the shell's listener
// answers is consumed there and never reaches the menu's accelerator. The
// pane chords follow the operator's stored set, which the shell reports and
// `menuBindings` resolves with the rules the shell's listener runs. The
// chords are this system's: macOS's own, or the Ctrl+Shift and Alt+Shift set
// Windows and Linux get from the same table (`systemRegistry`), so the menu shows the
// keys the window answers.

import type { MenuItemConstructorOptions } from "electron";
import type { TFunction } from "i18next";
import { commandTitle } from "../../../web/src/commandTitle";
import { CHECK_UPDATES_COMMAND, SETUP_COMMAND } from "../../../web/src/host";
import { AREA_COMMANDS, LANE_TOOL_COMMANDS, effectiveRegistry, isCycleCommand, isNumberedCommand, REGISTRY, systemRegistry, type Chord, type Command, type CommandId, type EffectiveRegistry, type KeySystem } from "../../../web/src/shortcuts";

const KEY_NAMES: Record<string, string> = {
  Enter: "Return",
  Backspace: "Backspace",
  Delete: "Delete",
  Tab: "Tab",
  Backquote: "`",
  Minus: "-",
  Equal: "=",
  Comma: ",",
  Period: ".",
  Slash: "/",
  Semicolon: ";",
  Quote: "'",
  BracketLeft: "[",
  BracketRight: "]",
  Backslash: "\\",
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
};

/** A registry chord as an Electron accelerator ("Shift+Command+T", "Control+Shift+T"). */
export function accelerator(chord: Chord): string {
  const key = chord.code.startsWith("Key")
    ? chord.code.slice(3)
    : chord.code.startsWith("Digit")
      ? chord.code.slice(5)
      : KEY_NAMES[chord.code];
  if (!key) throw new Error(`no accelerator for key code ${chord.code}`);
  return [chord.ctrl && "Control", chord.alt && "Alt", chord.shift && "Shift", chord.meta && "Command", key].filter(Boolean).join("+");
}

/**
 * Which commands each menu carries, in order; null is a separator. The
 * numbered ⌘1-9 / ⌥1-9 / ⌥⌘1-9 selections and Move to Trash stay keyboard-only.
 * Cycles have an immediate click form, with input owned by the renderer or
 * native page rather than an app-menu accelerator.
 */
export const MENU_LAYOUT: Readonly<Record<"app" | "File" | "Edit" | "View" | "Pane" | "Help", readonly (CommandId | null)[]>> = {
  app: ["settings"],
  File: ["new_tab", "start_agent", "new_workspace", "reopen_closed_tab", null, "save_file", null, "close_pane", "close_tab"],
  Edit: ["find_in_pane"],
  View: [
    "search",
    "open_file",
    null,
    "recent_area_tab", "previous_recent_area_tab",
    "recent_panel", "previous_recent_panel",
    "recent_project", "previous_recent_project",
    null,
    "toggle_left_sidebar",
    "toggle_sidebar_lens",
    "overview",
    "add_backlog",
    "sidebar_projects",
    "toggle_device_rail",
    "toggle_lane",
    ...LANE_TOOL_COMMANDS,
    "toggle_lane_documents",
    "open_project_memory",
    "open_sidebar_prs",
    null,
    "text_larger",
    "text_smaller",
    "text_reset",
  ],
  Pane: ["split_right", "split_down", "toggle_zoom", "toggle_plan", null, "keep_open", null, ...AREA_COMMANDS],
  Help: ["shortcuts"],
};

export const KEYBOARD_ONLY: readonly CommandId[] = [
  "move_to_trash",
  "copy_path",
  "copy_relative_path",
  "explorer_cut",
  "explorer_copy",
  "explorer_paste",
  ...REGISTRY.map((command) => command.id).filter(isNumberedCommand),
];

function commandItem(command: Command, send: (id: CommandId) => void, t: TFunction<"translation">): MenuItemConstructorOptions {
  // A command with no chord (the sidebar switch until the operator binds one) is a plain item.
  return { id: command.id, label: commandTitle(command.id, t), accelerator: command.electron && !isCycleCommand(command.id) ? accelerator(command.electron) : undefined, click: () => send(command.id) };
}

// A reported set is the editable commands the operator rebound; anything past
// these caps is not one. The core refuses a stored set past the same caps
// (`BINDINGS_CAP` and `BINDING_TEXT_CAP` in herdr-core/src/runtime/events.rs);
// change them together. The cap holds every editable command with room to
// spare: `menu.test.ts` fails when a new one would no longer fit.
export const BINDINGS_CAP = 32;
const BINDING_TEXT_CAP = 64;

/**
 * The registry the menu shows for a set the page reported, or why the report
 * was refused. The page is the shell hided serves, but what crosses the
 * bridge is checked here all the same: a plain map of short strings, then
 * the shell's own resolution, whose fallback to the defaults the menu shares.
 */
export function menuBindings(reported: unknown, system: KeySystem): EffectiveRegistry | { refused: string } {
  if (typeof reported !== "object" || reported === null || Array.isArray(reported)) return { refused: "not a map" };
  const entries = Object.entries(reported);
  if (entries.length > BINDINGS_CAP) return { refused: "too many entries" };
  const stored: Record<string, string> = {};
  for (const [key, value] of entries) {
    if (typeof value !== "string" || key.length > BINDING_TEXT_CAP || value.length > BINDING_TEXT_CAP) return { refused: "not short strings" };
    stored[key] = value;
  }
  return effectiveRegistry(stored, "electron", system);
}

function items(layout: readonly (CommandId | null)[], send: (id: CommandId) => void, registry: readonly Command[], t: TFunction<"translation">): MenuItemConstructorOptions[] {
  return layout.map((id) => {
    if (id === null) return { type: "separator" };
    const command = registry.find((row) => row.id === id);
    if (!command) throw new Error(`menu names an unknown command ${id}`);
    return commandItem(command, send, t);
  });
}

/** What a menu item sends the shell: a registry command, or the host's own Set up Hide and Check for Updates…. */
export type MenuCommand = CommandId | typeof SETUP_COMMAND | typeof CHECK_UPDATES_COMMAND;

export function menuTemplate(options: {
  appName: string;
  send: (id: MenuCommand) => void;
  developer: boolean;
  system: KeySystem;
  /** The language the menu is drawn in; a role item carries its label too, since the system's own follows the OS, not the operator's choice. */
  t: TFunction<"translation">;
  registry?: readonly Command[];
}): MenuItemConstructorOptions[] {
  const { appName, send, developer, system, t } = options;
  const registry = options.registry ?? systemRegistry(system);
  const mac = system === "mac";
  // Services and hiding the app are macOS's own; Windows and Linux have neither.
  const macOnly: MenuItemConstructorOptions[] = mac
    ? [
        { role: "services", label: t("native.menu.services") },
        { type: "separator" },
        { role: "hide", label: t("native.menu.hide", { appName }) },
        { role: "hideOthers", label: t("native.menu.hideOthers") },
        { role: "unhide", label: t("native.menu.showAll") },
        { type: "separator" },
      ]
    : [];
  // Electron gives quit, reload and the Window menu's close and minimize
  // plain Ctrl keys on Windows and Linux, which belong to the shell there
  // (operator decision 2026-10-03): quit and reload take the rule's
  // Ctrl+Shift, and the Window menu stays macOS's, since those windows close
  // and minimize from their own title bar. Only the edit roles keep plain Ctrl.
  const pcKeys = (accelerator: string) => (mac ? {} : { accelerator });
  // Where each system's apps keep it (PRD auto-update D-25): macOS's app menu, Windows' and Linux's Help.
  const checkForUpdates: MenuItemConstructorOptions = { id: CHECK_UPDATES_COMMAND, label: t("native.menu.checkForUpdates"), click: () => send(CHECK_UPDATES_COMMAND) };
  return [
    {
      label: appName,
      submenu: [
        { role: "about", label: t("native.menu.about", { appName }) },
        ...(mac ? [checkForUpdates] : []),
        { type: "separator" },
        ...items(MENU_LAYOUT.app, send, registry, t),
        { type: "separator" },
        ...macOnly,
        { role: "quit", label: t("native.menu.quit", { appName }), ...pcKeys("Control+Shift+Q") },
      ],
    },
    { label: t("native.menu.file"), submenu: items(MENU_LAYOUT.File, send, registry, t) },
    {
      label: t("native.menu.edit"),
      submenu: [
        { role: "undo", label: t("native.menu.undo") },
        { role: "redo", label: t("native.menu.redo") },
        { type: "separator" },
        { role: "cut", label: t("native.menu.cut") },
        { role: "copy", label: t("native.menu.copy") },
        { role: "paste", label: t("native.menu.paste") },
        { role: "pasteAndMatchStyle", label: t("native.menu.pasteMatchStyle") },
        { role: "selectAll", label: t("native.menu.selectAll") },
        { type: "separator" },
        ...items(MENU_LAYOUT.Edit, send, registry, t),
      ],
    },
    {
      label: t("native.menu.view"),
      submenu: [
        ...items(MENU_LAYOUT.View, send, registry, t),
        ...(developer
          ? ([
              { type: "separator" },
              { role: "reload", label: t("native.menu.reload"), ...pcKeys("Control+Shift+R") },
              { role: "toggleDevTools", label: t("native.menu.developerTools") },
            ] as MenuItemConstructorOptions[])
          : []),
        { type: "separator" },
        { role: "togglefullscreen", label: t("native.menu.fullScreen") },
      ],
    },
    { label: t("native.menu.pane"), submenu: items(MENU_LAYOUT.Pane, send, registry, t) },
    ...(mac
      ? ([
          {
            role: "windowMenu",
            label: t("native.menu.window"),
            submenu: [
              { role: "minimize", label: t("native.menu.minimize") },
              { role: "zoom", label: t("native.menu.zoom") },
              { type: "separator" },
              { role: "front", label: t("native.menu.front") },
            ],
          },
        ] as MenuItemConstructorOptions[])
      : []),
    {
      role: "help",
      label: t("native.menu.help"),
      submenu: [
        { id: SETUP_COMMAND, label: t("native.menu.setupHide"), click: () => send(SETUP_COMMAND) },
        ...(mac ? [] : [checkForUpdates]),
        { type: "separator" },
        ...items(MENU_LAYOUT.Help, send, registry, t),
      ],
    },
  ];
}
