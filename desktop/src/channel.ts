/** App-menu command ids, main -> renderer (B11). */
export const COMMAND_CHANNEL = "hide:command";

/** The stored macOS pane chords the menu is built from, renderer -> main. */
export const BINDINGS_CHANNEL = "hide:bindings";

/** The core's explicit interface language (a language code or null for the system's), renderer -> main. */
export const LANGUAGE_CHANNEL = "hide:language";

/** A file or folder the shell asks the OS file manager to show (a menu's `reveal_external`), renderer -> main. */
export const REVEAL_CHANNEL = "hide:reveal";

/**
 * Terminal links (docs/ARCHITECTURE.md, A clicked path is one event): which
 * paths a link names exist on this Mac and what they are (invoke), and a
 * path handed to macOS, opened in its default application or revealed in
 * Finder when opening would run it (renderer -> main).
 */
export const PROBE_PATHS_CHANNEL = "hide:probe-paths";
export const OPEN_PATH_CHANNEL = "hide:open-path";

/**
 * System notifications (PRD first-run-onboarding D-10): the shell asks
 * for a notification about one agent
 * (renderer -> main), and a click on it hands that id back (main -> renderer).
 */
export const NOTIFY_CHANNEL = "hide:notify";
export const NOTIFY_OPEN_CHANNEL = "hide:notify-open";

/**
 * Agent notifications (PRD first-run-onboarding D-10, D-12): the shell
 * withdraws notifications about agents the operator has seen
 * (renderer -> main), asks whether macOS lets Hide show any (invoke), and
 * opens Hide's page of the system's notification settings (renderer -> main).
 */
export const NOTIFY_WITHDRAW_CHANNEL = "hide:notify-withdraw";
export const NOTIFY_PERMISSION_CHANNEL = "hide:notify-permission";
export const NOTIFY_SETTINGS_CHANNEL = "hide:notify-settings";

/**
 * The app icon's badge: how many root agents need the operator, which the
 * shell reports once it holds a live snapshot and again only when the number
 * changes (renderer -> main).
 */
export const NOTIFY_BADGE_CHANNEL = "hide:notify-badge";

/** Add a project's Browse folder: the native folder picker, answered with the chosen folder or null (invoke). */
export const PICK_FOLDER_CHANNEL = "hide:pick-folder";

/**
 * Browser displays (issue 155): the shell reports where each browser display
 * of the front Workspace sits (renderer -> main), asks for a still of one
 * (invoke), sends toolbar commands, and hears each page's state back
 * (main -> renderer). Nothing else crosses between the shell and a page.
 */
export const BROWSER_SYNC_CHANNEL = "hide:browser-sync";
export const BROWSER_CAPTURE_CHANNEL = "hide:browser-capture";
export const BROWSER_COMMAND_CHANNEL = "hide:browser-command";
export const BROWSER_EVENT_CHANNEL = "hide:browser-event";
export const BROWSER_CYCLE_END_CHANNEL = "hide:browser-cycle-end";

/**
 * The window frame the shell's top row is drawn inside (PRD
 * sidebar-titlebar-refresh D-15, D-17): whether macOS draws its window
 * buttons over the row and whether the window fills the screen. The shell
 * asks once (invoke) and hears every change (main -> renderer).
 */
export const WINDOW_CHROME_GET_CHANNEL = "hide:window-chrome-get";
export const WINDOW_CHROME_CHANNEL = "hide:window-chrome";

/**
 * Open in VS Code (PRD sidebar-titlebar-refresh D-13, D-14): whether VS Code
 * is installed on this computer (invoke), and a folder handed to it
 * (renderer -> main).
 */
export const EDITOR_AVAILABLE_CHANNEL = "hide:editor-available";
export const EDITOR_OPEN_CHANNEL = "hide:editor-open";

/**
 * This app's own updates (PRD auto-update): the shell asks for the status
 * once (invoke) and hears every change (main -> renderer); it asks for a
 * check, a Restart into the ready update, and preview releases on or off
 * (renderer -> main).
 */
export const UPDATE_STATUS_GET_CHANNEL = "hide:update-status-get";
export const UPDATE_STATUS_CHANNEL = "hide:update-status";
export const UPDATE_CHECK_CHANNEL = "hide:update-check";
export const UPDATE_RESTART_CHANNEL = "hide:update-restart";
export const UPDATE_PRERELEASE_CHANNEL = "hide:update-prerelease";

/**
 * Restarting the operator's Herdr server on this app's Herdr where Herdr
 * cannot hand it over live (PRD herdr-takeover B12): the shell asks once the
 * operator confirmed what ends (invoke), and the host answers how it went.
 */
export const HERDR_RESTART_CHANNEL = "hide:herdr-restart";

/**
 * Quitting the app from the screen that holds Hide while this machine's Herdr
 * speaks another protocol (PRD herdr-takeover): the window's own Quit, as
 * the app menu's would; hided keeps running, as it does after any quit.
 */
export const QUIT_CHANNEL = "hide:quit";
