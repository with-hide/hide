// Which host runs the shell: a browser tab, or the desktop app
// (`desktop/`), whose preload exposes `window.hideHost` and nothing else.
// The bridge carries the app menu's commands in, the operator's macOS pane
// chords out, so the menu shows and answers the chords the page runs, the
// host's OS, a file or folder to show in the OS file manager, the paths a terminal link names and handing one
// to macOS, places the pages of browser displays (issue 155), and shows the
// Factory's notifications with their clicks back; the shell
// never reaches the host any other way (desktop PRD B11).

import { revealLabel, type RevealHost } from "./revealExternal";
import { holdsFieldModifier, keySystemOf, type KeySystem } from "./shortcuts";

export type HostKind = "browser" | "electron";

/** A browser display's place in the window, in the shell's CSS pixels. */
export type BrowserRect = { x: number; y: number; width: number; height: number };

/** One browser display of the front Workspace as the host places it. */
export type BrowserPlacement = {
  /** The display id; unique within `BrowserSync.workspace` only. */
  id: string;
  /** The authoritative View area containing this display. */
  area_id: string;
  url: string;
  /** The core's load stamp: a larger one than the page last loaded loads `url` again. */
  load: number;
  /** Where its page shows; null while it is not on screen, which keeps the page alive but hidden. */
  rect: BrowserRect | null;
  /** False while a still or a notice stands in its place. */
  visible: boolean;
  /**
   * The device size its page lays out at (PRD browser-width-zoom D-10, D-12):
   * `rect` is then the page's own rectangle, centered or fitted in its slot,
   * and the page's viewport is `width` CSS pixels wide, scaled to fill
   * `rect`; `mobile` lays it out as a phone or tablet does (its meta
   * viewport applies). Null, or absent from an older shell, fits the slot.
   */
  device?: BrowserDevicePlacement | null;
  /**
   * An outside HTML file's grant (PRD outside-path-in-hide): the host loads
   * the file it granted, and `url` is empty, since the path never leaves the
   * host.
   */
  grant?: string;
};

/** A device size as the host emulates it. */
export type BrowserDevicePlacement = { width: number; mobile: boolean };

/**
 * Every browser display of the front Workspace. The host closes a page of
 * that Workspace the list no longer names, hides the pages of any other,
 * and creates a page only once its display is on screen.
 */
export type BrowserSync = {
  workspace: string | null;
  displays: BrowserPlacement[];
  /** Core-owned inventory; a hidden native page absent here is closed. */
  retained: { workspace: string; id: string; area_id: string }[];
  /** Changes on controller mount or reconnect so the host replays actual attachment facts once. */
  attachment_epoch?: string;
  /** Positive core area authority, including empty areas; absent grants no debugger access. Incarnation changes on revoke/regrant. */
  authorized_scopes?: { workspace: string; area_id: string; incarnation: number }[];
  /** The core's own node id, whose pages load this computer's loopback directly; absent until the snapshot names it. */
  node?: string;
  /**
   * Every outside-file grant a tab stands on, the core's and the opens still
   * on their way; the host lets go of every other one. Absent until the shell
   * has the core's tabs, so a reload lets go of nothing (PRD outside-path-in-hide B15).
   */
  external_grants?: string[];
};

/** `zoom_in`, `zoom_out` and `zoom_reset` step the page's zoom the way Chrome's ⌘= / ⌘- / ⌘0 do, without moving the keyboard. */
export type BrowserCommand = "back" | "forward" | "reload" | "stop" | "focus" | "zoom_in" | "zoom_out" | "zoom_reset";

/** What the page itself says, which only the host can read. */
export type BrowserPageState = {
  url: string;
  title: string;
  loading: boolean;
  canGoBack: boolean;
  canGoForward: boolean;
  /** Why the last load failed or the page stopped; null while it shows. */
  failure: string | null;
  /**
   * The page's zoom factor, 1 for 100%. Chromium keeps one per host of a
   * session, so a zoom anywhere is reported again for every page of that
   * host. Absent from an older host.
   */
  zoom?: number;
};

export type BrowserHostEvent =
  | { kind: "window-focus"; focused: boolean }
  /** `external` marks an outside file's page, whose address is a path the core never holds (PRD outside-path-in-hide). */
  | { kind: "state"; workspace: string; id: string; load: number; state: BrowserPageState; external?: true }
  /** Actual debugger ownership, scoped to `device_id + NUL + checkout_path`. */
  | { kind: "attached"; workspace: string; id: string; attached: boolean }
  | { kind: "gone"; workspace: string; id: string; load: number; url: string; external?: true }
  /** The page opened a tab: it becomes another browser display, beside the page `id`. */
  | { kind: "open"; workspace: string; id: string; url: string }
  /** The operator clicked or tabbed into the page. */
  | { kind: "focus"; workspace: string; id: string }
  | { kind: "cycle-input"; cycleId: number; workspace: string; id: string; type: "keyDown" | "keyUp"; key: string; code: string; control: boolean; alt: boolean; meta: boolean; shift: boolean }
  | { kind: "cycle-cancel"; cycleId: number; workspace: string; id: string };

export type BrowserBridge = {
  sync(state: BrowserSync): void;
  /** The shell ended a held cycle, including a release delivered to the shell after hiding a page. */
  endCycle(cycleId: number): void;
  /** A still of the page as a data URL, or null when it has none to give. */
  capture(workspace: string, id: string): Promise<string | null>;
  command(workspace: string, id: string, command: BrowserCommand): void;
  onEvent(listener: (event: BrowserHostEvent) => void): () => void;
};

/** How a Herdr restart went: `stop_failed` changed nothing; `unavailable` is a host with no bundled Herdr. */
export type HerdrRestart = "restarted" | "stop_failed" | "failed" | "unavailable" | "refused";

/** A path on this computer as the host found it: its physical path in the wire spelling (`/` between names, `C:/...` on Windows) and whether it is a folder; null when it does not exist. */
export type ProbedPath = { real: string; kind: "file" | "directory" } | null;

/** Which viewer an outside file opens in (PRD outside-path-in-hide D-02). */
export type ExternalKind = "text" | "image" | "pdf" | "video" | "html";
/** An outside file this window's host granted: never its path, which only `describe` gives this window. */
export type ExternalGrant = { id: string; name: string; kind: ExternalKind };
/** A granted file's read: its bytes, a text past the tab's size, or why it could not be read. */
export type ExternalRead = { bytes: Uint8Array } | { preview_only: true } | { failure: string };

/** The outside files this window may show (PRD outside-path-in-hide); only a desktop window has them. */
export type ExternalBridge = {
  /**
   * Grants the clicked file at `path` (its physical wire spelling) to this
   * window for its viewer, `source` asking for an HTML file's text; null
   * when the host handed it to the system instead (a program, audio, a
   * folder, past the size, or the grants are full).
   */
  open(path: string, mode: "auto" | "source"): Promise<ExternalGrant | null>;
  /** The granted file's bytes, read from the disk now. */
  read(grant: string): Promise<ExternalRead>;
  /** The granted file's path and kind for this window's badge; null when this window's host does not hold it. */
  describe(grant: string): Promise<{ path: string; name: string; kind: ExternalKind } | null>;
  /** Every grant the host holds now. */
  live(): Promise<string[]>;
};

export type HostBridge = {
  kind: "electron";
  /** The host's OS as Node names it (`process.platform`): what the OS file manager is called. */
  platform: string;
  /** Delivers each app-menu command id, a registry command's or `SETUP_COMMAND`; returns the unsubscribe. */
  onCommand(listener: (id: string) => void): () => void;
  /** Hands the host the stored macOS pane chords (`ui_state.shortcut_bindings`) it builds the menu from. */
  reportBindings(bindings: Record<string, string>): void;
  /** Hands the host the core's explicit interface language (`ui_state.interface_language`, null for the system's) once a snapshot confirms it; the app menu and status page are drawn in it. */
  reportLanguage(language: string | null): void;
  /** Shows a file or folder of this computer, named in the wire spelling, in the OS file manager, selected in its parent folder; nothing is opened. */
  revealPath(path: string): void;
  /** The native folder picker, modal to the window; the chosen folder in the wire spelling, or null when the operator cancelled. */
  pickFolder(): Promise<string | null>;
  /** What each absolute (in the wire spelling) or `~/` path names on this computer, in order; at most `MAX_PROBE_PATHS` (64) per call. */
  probePaths(paths: string[]): Promise<ProbedPath[]>;
  /** Hands an absolute path, in the wire spelling, to the system: its default application, a folder window, or a file manager reveal when opening would run it. */
  openPath(path: string): void;
  /** Outside files shown in Hide; absent from an older host. */
  external?: ExternalBridge;
  /**
   * Shows a system notification about one Factory item (PRD factory-observer
   * D-50) or agent (PRD first-run-onboarding D-10) while the app runs; a
   * newer one with the same id replaces it. With `unlessFocused` the host
   * drops it while a Hide window has the keyboard.
   */
  notify(id: string, title: string, body: string, options?: { unlessFocused?: boolean }): void;
  /** Delivers the id of a notification the operator clicked; returns the unsubscribe. */
  onNotificationOpen(listener: (id: string) => void): () => void;
  /** Closes the shown notifications with these ids (an agent the operator has seen); at most 64. */
  withdrawNotifications(ids: string[]): void;
  /** Whether the system lets Hide show notifications; read on each call, at most one read at a time. */
  notificationPermission(): Promise<NotificationPermission>;
  /** Opens Hide's page of the system's notification settings (macOS). */
  openNotificationSettings(): void;
  /**
   * Hands the host how many root agents are in enabled notification groups, for the app icon
   * (the Dock badge, the Windows taskbar overlay); zero clears it. Reported
   * once the page holds a live snapshot, which is also when the host hands
   * over a notification click that started the app, and then on each change.
   */
  reportBadge(count: number): void;
  /**
   * The window frame the shell draws its top row inside (PRD
   * sidebar-titlebar-refresh D-15, D-17): delivered once at once and again on
   * every change; returns the unsubscribe.
   */
  onWindowChrome(listener: (chrome: WindowChrome) => void): () => void;
  /** Whether VS Code is installed on this computer, as the host found it once per launch (D-14). */
  editorAvailable(): Promise<boolean>;
  /**
   * Opens a folder of this computer, named in the wire spelling, as a VS Code
   * window; the host logs the outcome without the path and shows nothing (B16).
   */
  openInEditor(path: string): void;
  /**
   * Restarts the operator's Herdr server on this app's Herdr where Herdr
   * cannot hand it over live (PRD herdr-takeover B12), once the operator
   * confirmed what ends; a second call while one runs answers with it.
   * `failed` has already put the start failure on the window's status page.
   */
  restartHerdr(): Promise<HerdrRestart>;
  /** Quits the app, as the app menu's Quit does; hided keeps running. */
  quit(): void;
  browser: BrowserBridge;
  /** This app's own updates (PRD auto-update), which only the desktop host has. */
  updates: UpdatesBridge;
};

/**
 * Where this app's update stands (PRD auto-update D-17 to D-22), as the
 * desktop host's updater reports it:
 * - `off`: this app does not update itself: a development or CI build, or an
 *   app not installed from a package (`development`, B23), or a macOS app in
 *   a place it cannot replace itself, such as the disk image (`location`, B27);
 * - `idle`: nothing newer; `checkedAt` is the last check that finished, in ms;
 * - `checking`, `downloading` (with its version and 0-100 percent);
 * - `ready`: downloaded and verified, waiting for a restart or the next quit;
 * - `failed`: the last `check` or `download` (which includes its sha512 and,
 *   on macOS, signature check) failed; the reason is in the host's log.
 */
export type UpdatePhase =
  | { kind: "off"; reason: "development" | "location" }
  | { kind: "idle"; checkedAt: number | null }
  | { kind: "checking" }
  | { kind: "downloading"; version: string; percent: number }
  | { kind: "ready"; version: string }
  | { kind: "failed"; stage: "check" | "download" };

export type UpdateStatus = {
  phase: UpdatePhase;
  /** Whether preview releases (`-rc.N`) are taken too; this installation's choice, off at first (D-22). */
  prerelease: boolean;
  /** Whether a ready update installs when the app quits; Linux asks for the system password, so only on Restart (D-18). */
  installOnQuit: boolean;
};

export type UpdatesBridge = {
  /** Delivers the status at once and on every change; returns the unsubscribe. */
  onStatus(listener: (status: UpdateStatus) => void): () => void;
  /** Checks now, as Check for Updates… or About's button does. */
  check(): void;
  /** Installs the ready update and opens the new version; the core and its agents keep running (D-19). */
  restart(): void;
  /** Takes preview releases from the next check on, or stops taking them; never installs a lower version (D-22). */
  setPrerelease(on: boolean): void;
};

/**
 * What the window's own frame takes of the shell's top row: `trafficLights`
 * while macOS draws its window buttons over the row's left end (the title
 * bar is hidden only there), and `fullscreen` while the window fills the
 * screen, where macOS takes those buttons away.
 */
export type WindowChrome = { trafficLights: boolean; fullscreen: boolean };

/**
 * The app-menu commands the desktop host alone has, outside the shortcut
 * registry, so neither a browser tab's palette nor Settings > Shortcuts
 * offers them: Help > Set up Hide reopens the first-run wizard (PRD
 * first-run-onboarding B45).
 */
export const SETUP_COMMAND = "setup_hide";

/**
 * The app menu's Check for Updates… (PRD auto-update D-25): the host starts a
 * check and the shell opens Settings > About, where its outcome shows.
 */
export const CHECK_UPDATES_COMMAND = "check_updates";

/**
 * The system's notification permission for Hide (`desktop/src/main/notificationPermission.ts`):
 * `unsupported` where the system has none to read (Windows, Linux), `no_service`
 * where it cannot show a notification at all (a Linux desktop with no
 * notification service, PRD first-run-onboarding D-24), `unknown` where macOS
 * has one Hide could not read.
 */
export type NotificationPermission = "granted" | "denied" | "not_determined" | "unknown" | "unsupported" | "no_service";

declare global {
  interface Window {
    hideHost?: HostBridge;
  }
}

export function hostBridge(): HostBridge | null {
  return typeof window !== "undefined" && window.hideHost?.kind === "electron" ? window.hideHost : null;
}

/**
 * The keyboard convention of the machine the operator types on: the desktop
 * app's OS, or the browser's. The shell's chords, their glyphs and its
 * field chords all follow it (`shortcuts.ts`).
 */
export function keySystem(): KeySystem {
  const bridge = hostBridge();
  if (bridge) return keySystemOf(bridge.platform);
  if (typeof navigator === "undefined") return "pc";
  const agent = navigator as Navigator & { userAgentData?: { platform?: string } };
  return keySystemOf(agent.userAgentData?.platform || navigator.platform || navigator.userAgent);
}

/**
 * Whether a press or a click holds the system's own command key: ⌘ on macOS,
 * Ctrl elsewhere (`fieldChord`), as ⌘↵ in a palette does and as a click that
 * asks for the operating system rather than the shell does (Ctrl-click is
 * not the context menu off macOS).
 */
export function holdsCommandKey(event: { metaKey: boolean; ctrlKey: boolean }): boolean {
  return holdsFieldModifier(event, keySystem());
}

/** The OS file manager item's host: its label on this OS, or null in a plain browser tab, which has none. */
export function revealHost(): RevealHost {
  const bridge = hostBridge();
  return bridge ? { label: revealLabel(bridge.platform) } : null;
}

let editorFound = false;

/**
 * Asks the desktop host once per page whether VS Code is on this computer
 * (D-14); the row menus read the answer through `editorHost`. A browser tab
 * has no host to ask, and a refused question leaves the item out.
 */
export function readEditorAvailability(): void {
  const bridge = hostBridge();
  if (!bridge) return;
  bridge.editorAvailable().then(
    (found) => {
      editorFound = found;
    },
    () => {
      editorFound = false;
    },
  );
}

/** Whether the desktop host found VS Code on this computer. */
export function editorHost(): boolean {
  return editorFound;
}

export function hostKind(): HostKind {
  return hostBridge() ? "electron" : "browser";
}

/** The host's browser views, or null in a plain browser tab, which cannot draw a page in a View area. */
export function browserBridge(): BrowserBridge | null {
  return hostBridge()?.browser ?? null;
}
