// Browser displays (issue 155): one WebContentsView per browser display the
// shell has shown, placed over the rect the shell reports through the
// hideHost bridge. The core owns which displays exist and the URL each was
// asked to load; this owns the pages. A page lives in a browser session
// partition with no preload and no Node, so nothing in it can reach the
// shell's bridge or the daemon's token.
//
//   shown      -> created on first show, then placed and made visible
//   asked off screen -> a web page on no desk on screen whose load the core
//                 asks for: created and loaded hidden (PRD lane-desk-pair-view B32)
//   not shown  -> hidden, still alive; a move between areas never reloads
//   closed     -> the core's inventory no longer names it: destroyed,
//                 which ends its renderer process
//   closed itself -> the page's `window.close()` ended its contents: closed,
//                 and a fresh unloaded page on its slot says so until reloaded
//
// A hidden page past `MAX_LIVE_VIEWS`, or one hidden and idle for
// `IDLE_UNLOAD_MS`, is unloaded: closed, reported `gone`, and loaded again
// when shown or when an agent drives it (`cdpRevive`).
//
// A sized popup a page on screen opens (a sign-in window) is a real child
// window that keeps its opener, belongs to that page and closes with it; a
// link to another app leaves only after the operator agrees.

import { app, BrowserWindow, dialog, ipcMain, Menu, screen, session, shell, webContents as allContents, WebContentsView, type BrowserWindowConstructorOptions, type Input, type IpcMainEvent, type IpcMainInvokeEvent, type MenuItem, type Session, type WebContents, type WindowOpenHandlerResponse } from "electron";
import fs from "node:fs";
import path from "node:path";
import type { TFunction } from "i18next";
import { fileURLToPath } from "node:url";
import type { BrowserDevicePlacement, BrowserHostEvent, BrowserPageState, BrowserPlacement, BrowserRect, BrowserSync } from "../../../web/src/host";
import { hostChord, isCycleCommand, keySystemOf, matchHost, releaseModifier, systemRegistry, type Command, type CommandId } from "../../../web/src/shortcuts";
import { BROWSER_CAPTURE_CHANNEL, BROWSER_COMMAND_CHANNEL, BROWSER_CYCLE_END_CHANNEL, BROWSER_EVENT_CHANNEL, BROWSER_SYNC_CHANNEL, REMOTE_BROWSER_EVENT_CHANNEL } from "../channel";
import { appScheme, browserPartition, deviceEmulation, fileLink, IDLE_CHECK_MS, IDLE_UNLOAD_MS, idleExpired, isLocalNetworkHost, isPopup, loadable, localNetworkReach, localPageRequest, MAX_AUTHORIZED_SCOPES, MAX_LIVE_VIEWS, MAX_POPUPS, offScreenBounds, offScreenEmulation, overCap, parseCommand, parseRemoteSync, parseSync, parseTarget, placeIn, popupBounds, PRESS_WITHIN_MS, protocolOf, REMOTE_BOUNDS, REMOTE_FILE_PARTITION, REMOTE_PAGE_PARTITION, remoteRequest, remoteRouteAddress, routed, sameFileAddress, toBounds, viewKey, wireFileAddress } from "./browserSync";
import { isLoopbackHost } from "../../../web/src/loopbackHost";
import { nextZoomFactor, type PageZoom } from "../../../web/src/zoomSteps";
import type { HostLog } from "./log";
import type { CdpPage, CdpPurpose, CdpRetirement, CdpScope } from "./browserCdp";
import { accelerator } from "./menu";
import { errorCode, externalPartition, type ExternalFiles } from "./externalFiles";
import { egressWords } from "./remoteEgress";


/** Page state reports coalesce to one per view in this window, so a title ticking every frame costs one event. */
const REPORT_COALESCE_MS = 100;
/** How long an agent's connection waits for an unloaded page's load, inside hided's 8 s gateway timeout. */
const REVIVE_WAIT_MS = 5_000;
/** Chromium's code for a load a newer one replaced; not a failure. */
const ERR_ABORTED = -3;
const PAGE_PERMISSIONS = new Set(["clipboard-sanitized-write"]);
/** The text-size commands, which zoom a focused page the way Chrome's ⌘= / ⌘- / ⌘0 do. */
const PAGE_ZOOM_COMMANDS: Readonly<Partial<Record<CommandId, PageZoom>>> = { text_larger: "in", text_smaller: "out", text_reset: "reset" };
/** The shell's zoom commands for one page, from the zoom badge's popover. */
const SHELL_ZOOM_COMMANDS: Readonly<Record<string, PageZoom>> = { zoom_in: "in", zoom_out: "out", zoom_reset: "reset" };
/** Chrome's second zoom-in chord, ⌘+ (⌘⇧= on a US keyboard), as an app-menu accelerator. */
const ZOOM_IN_ALIAS = accelerator({ code: "Equal", meta: true, shift: true });
/** Pinch zoom on a page, as visual zoom levels; Electron turns it off by default. */
const PINCH_ZOOM_LIMITS = [1, 3] as const;
/** One first-refusal witness per native generation and guard category. */
const FILE_REFUSAL_EVENTS = {
  request: "browser.request_refused", navigation: "browser.navigation_refused", popup: "browser.window_open_refused",
  local_file: "browser.request_refused", local_network: "browser.request_refused",
} as const;
type FileRefusal = keyof typeof FILE_REFUSAL_EVENTS;

/** Input that presses in a page, as `input-event` names it. */
const PRESSES = new Set(["mouseDown", "touchStart", "gestureTapDown"]);

type Page = {
  key: string;
  /** Who placed it: this window's shell, or another computer's screen in it. */
  readonly source: Source;
  workspace: string;
  id: string;
  view: WebContentsView;
  /** Its contents' id, kept because a view whose contents were destroyed no longer names them. */
  readonly contentsId: number;
  /** The load stamp this page last loaded. */
  applied: number;
  visible: boolean;
  /** Placed on its slot in the front Workspace, shown or standing behind its still for an overlay. */
  placed: boolean;
  shownAt: number;
  /** The last time the page was on screen or held by something that would break if it closed. */
  idleSince: number;
  /** Media plays in it, heard or not. */
  playing: boolean;
  /** Its downloads in progress, its popups' included. */
  downloads: number;
  /** Called once its load settles, for the agents' connections waiting on it; at most `MAX_HTTP_REQUESTS` of the gateway's. */
  readonly settled: Set<() => void>;
  state: BrowserPageState;
  /** The device size the page lays out at, null to fit its slot, with the bounds it was last placed at. */
  device: BrowserDevicePlacement | null;
  bounds: BrowserRect | null;
  /** The emulation last applied to its contents, as JSON; null when none is. */
  emulated: string | null;
  report: NodeJS.Timeout | null;
  route: ResolvedPage | null;
  /** The load the core asked for that has not settled yet; a newer one replaces it. */
  loading: CoreLoad | null;
  partition: string;
  /** Native file reads are forbidden before the first load of a web/blank
   * generation. Only a separately created manual-file partition permits them. */
  readonly fileRestricted: boolean;
  /** CDP can install future scripts. Its filesystem-write/download ban
   * survives disconnect until this native generation ends. */
  cdpRestricted: boolean;
  /** Download refusals remain enforced after disconnect, with one diagnostic
   * for this native generation even when installed scripts loop. */
  cdpDownloadRefused: boolean;
  /** A generation that has held a native file frame never enters CDP. */
  cdpFileFrame: boolean;
  /** A file page an agent's file capability has leased keeps its own
   * documents in its checkout until this native generation ends
   * (`localPageRequest`), whatever moves it. */
  cdpFileHeld?: boolean;
  /** The last address `cdpPages` asked about and whether it is a file of the
   * page's checkout, since the gateway asks on every command. */
  cdpShown?: { url: string; inside: boolean };
  /** Three finite guard categories bound diagnostic state and output even
   * when a renderer repeatedly attempts file requests or opens windows. */
  readonly fileRefusals: Set<FileRefusal>;
  /** A local file page's checkout, resolved once per route: what its files may load from. */
  checkoutRoot?: string | null;
  /**
   * An outside HTML file's grant (PRD outside-path-in-hide): the page loads
   * the granted file in a session of its own and its files from the grant's
   * folder alone, and it never enters CDP or reports its address to the core.
   */
  readonly grant?: string;
  /** What each public name a local file page asked for resolved to, bounded. */
  readonly hostVerdicts: Map<string, boolean>;
  /** Its document navigation in flight that began while something else held the keyboard. */
  grab: NavigationGrab | null;
  /**
   * A page of another computer's screen: whether its route said it is that
   * computer's own page, whose loopback it may reach through that computer,
   * and the last time the operator pressed in it or its popups, which a
   * window it opens needs (docs/BROWSER_DISPLAYS.md, A page in another
   * computer's screen).
   */
  own?: boolean;
  /**
   * A remote screen's page: the address its screen's last sync named for its
   * display, what that computer's core holds now (a page's own move inside
   * itself included, once its report reached the core), which its Reload
   * asks for.
   */
  address?: string;
  pressedAt?: number;
  /** The window's count of views added when this page was added last, so it is raised only above a view added after it. */
  addedAt: number;
  /** A download this generation refused, logged once. */
  downloadRefused?: boolean;
  /** The reasons a remote screen's page had a request refused, each logged once. */
  readonly remoteRefusals: Set<string>;
  /**
   * Laid out at `bounds` by emulation because no slot has shown it since its
   * contents were made: Chromium gives a page it never showed no viewport, so
   * an agent's clicks would land nowhere (PRD lane-desk-pair-view B32). Its
   * slot ends it.
   */
  offScreen?: boolean;
};
/**
 * Chromium hands the keyboard to a page's view as each new document commits,
 * whoever held it: a page that reloads itself (a dashboard's meta refresh) or
 * that an agent navigates would take the keys the operator is typing into a
 * terminal. Every document navigation of a page is one, from its start to its
 * stop. Whether the page's focus during it is a grab is decided when the focus
 * arrives, by who holds the keyboard then (`BrowserViews.keyboard`): the shell
 * or another page gets it back, the page itself keeps it, and a press in the
 * page while it loads (`claimed`) keeps it there. A decision at the start
 * would miss a keyboard that moved to the shell during the load.
 */
type NavigationGrab = { claimed: boolean };
/**
 * One load the core asked of a page, so its stages are logged against it
 * alone: `native` once its own `loadURL` was called, since until then a
 * commit or stop belongs to the load before it, and `committed` after its
 * first document commit. Each stage is one log line, so a load that never
 * ends names the stage it stopped at.
 */
type CoreLoad = { readonly load: number; readonly at: number; native: boolean; committed: boolean };

/** Names a local file page may have resolved at once; past it a name is resolved on every request. */
const MAX_HOST_VERDICTS = 64;

/**
 * A page's route: `root`, for a file page, is the checkout that holds its
 * file, even when another checkout's Workspace shows it: a local page loads
 * its files from there alone, and a route's addresses are read against it.
 */
export type ResolvedPage = { url: string; source_url: string; load: number; root?: string };

/** Why a page closes: a CDP retirement, or `idle` for one unloaded past `IDLE_UNLOAD_MS` (retired as `evicted`). */
type Closing = CdpRetirement["reason"] | "idle";

/**
 * What an unloaded web page needs to load again hidden for an agent: one per
 * display the core still retains, so the core's inventory bounds them.
 */
type Unloaded = { workspace: string; id: string; area_id: string; url: string; load: number; device: BrowserDevicePlacement | null; bounds: BrowserRect | null };

/** The idle bound and its check, with the clock both read; tests shorten them. */
export type IdleClock = { bound: number; every: number; now: () => number };

/** A popup window a page opened, under the page that owns it (for a popup's own popup, the same page). */
type Popup = { window: BrowserWindow; owner: Page };

/**
 * Where a page of another computer's screen loads from, as that computer's
 * core answers (`/page-route`): its address and whether it is that
 * computer's own page, or why it does not load here.
 */
export type RemoteRoute = { url: string; source_url: string; load: number; own: boolean; root?: string } | { refused: string };

/**
 * Another computer's screen in this window, as its pages need it
 * (docs/BROWSER_DISPLAYS.md, A page in another computer's screen): its page
 * places the browser displays of that computer, which this window draws in
 * the screen's tile with every request leaving through that computer.
 */
export type RemoteSource = {
  readonly device: string;
  readonly key: string;
  /** The screen's view contents: its page hears its pages' events, and the keyboard goes back to it. */
  contents(): WebContents | null;
  /** The tile in the window, with the zoom its page lays out at, while it shows its loaded page; null otherwise. */
  area(): { bounds: BrowserRect; zoom: number } | null;
  /** Whether the screen's own view holds the keyboard. */
  focused(): boolean;
  /** Whether its pages' requests may leave now: their proxy is set. */
  ready(): boolean;
  /** Asks for its proxy: `ready`, or why its pages cannot load now. */
  ensure(): Promise<string>;
  /** The partition its pages keep their logins in now (`remotePagePartition`); null before its proxy's first answer. */
  partition(): string | null;
  /** The partition its file pages load in now (`remoteFilePartition`); null before its proxy's first answer. */
  filePartition(): string | null;
  /** Whether its login storage may hold pages now; `waiting` until it can be judged, `unrecorded` when its row could not be written. */
  storage(): "live" | "clearing" | "full" | "waiting" | "unrecorded";
  route(workspace: string, id: string, load: number): Promise<RemoteRoute>;
  /** Lets go of the route that computer made for a page that closed. */
  release(workspace: string, id: string, load: number): void;
};

/** Whose pages a page is among: this window's shell, or one remote screen. */
type Source = {
  readonly id: string;
  readonly remote: RemoteSource | null;
  /** The core's own node id as this source's shell last named it. */
  node: string | null;
  /**
   * The newest load stamp of the core's inventory this source's shell has
   * sent, or null before its first inventory. The core gives each load a
   * stamp later than every earlier one, so a retained page with no live page
   * here and a newer stamp is one the core asked to load after this shell saw
   * it (B32). The first inventory sets it and asks nothing, so a relaunch, a
   * reload or a checkout that comes back loads none of the pages it already
   * had. Only this window's shell's pages load off screen.
   */
  newestLoad: number | null;
  /** A remote screen's last sync, applied while it shows and at most every `syncEveryMs`. */
  latest: BrowserSync | null;
  syncTimer: NodeJS.Timeout | null;
  syncedAt: number;
  /** When each of its recent pages was made, and the wait for the next place. */
  created: number[];
  createTimer: NodeJS.Timeout | null;
  capturing: boolean;
  captured: number[];
  commanded: number[];
  commandsRefused: boolean;
  /** When its pages last opened a popup. */
  popupAt: number;
  /** The proxy is being asked for; its next sync applies once it answers. */
  egressAsked: boolean;
  /** The reasons a request with no page (a service worker) was refused, each logged once. */
  readonly refusals: Set<string>;
  /** The screen hid or failed: its popups are hidden until it shows again. */
  hidden: boolean;
};

function newSource(id: string, remote: RemoteSource | null): Source {
  return { id, remote, node: null, newestLoad: null, latest: null, syncTimer: null, syncedAt: 0, created: [], createTimer: null, capturing: false, captured: [], commanded: [], commandsRefused: false, popupAt: 0, egressAsked: false, refusals: new Set(), hidden: false };
}

/** A remote source's id, from its machine and key. */
function remoteId(remote: { device: string; key: string }): string {
  return `${remote.device}\u0000${remote.key}`;
}

/** The key a page lives under: this window's by its display (`viewKey`), a remote screen's apart from it. */
function pageKey(source: Source, workspace: string, id: string): string {
  return source.remote ? `\u0001${source.id}\u0001${viewKey(workspace, id)}` : viewKey(workspace, id);
}

/** Entries of a list of times newer than `within` before `now`. */
function recent(times: number[], now: number, within: number): number[] {
  return times.filter((time) => now - time < within);
}

/** The most pixels of a phone's still of a page; a larger one is scaled down, so it stays well under the phone's 10 MiB image cap. */
const STILL_PIXELS = 1_500_000;
const STILL_QUALITY = 70;
/**
 * How long a still's capture may take before the gateway answers without it:
 * under hided's 8 s request bound, so a capture that never answers frees its
 * gateway request slot instead of holding it until the app quits.
 */
const STILL_WAIT_MS = 5_000;

export class BrowserViews {
  private readonly pages = new Map<string, Page>();
  /** Unloaded web pages by key, until their display closes or they show again. */
  private readonly unloaded = new Map<string, Unloaded>();
  private idleTimer: NodeJS.Timeout | null = null;
  /** Area membership comes from the core's retained inventory, including hidden workspaces. */
  private cdpAreas = new Map<string, string>();
  private cdpChanged: ((retirement?: CdpRetirement) => void) | null = null;
  private cdpObserved: { key: string; area: string; contents: number; url: string; title: string }[] = [];
  private readonly cdpLeases = new Map<number, CdpPage>();
  private attachmentEpoch: string | null = null;
  /** This window's shell's pages; its `node` is the core's own node id, whose pages load this computer's loopback directly. */
  private readonly local: Source = newSource("", null);
  /** Each remote screen's pages, by `remoteId`. */
  private readonly sources = new Map<string, Source>();
  /** Views added to the window by another owner (a remote screen); a page added before the latest is raised when placed. */
  private viewsAdded = 0;
  /** Positive core authority is independent of whether an area has pages. */
  private cdpScopes = new Map<string, number>();
  private cdpScopesRevision = 0;
  private cdpObservedRevision = -1;
  /** Popup windows by their contents' id. */
  private readonly popups = new Map<number, Popup>();
  private readonly configuredSessions = new Set<string>();
  private window: BrowserWindow | null = null;
  /** The question open about a link to another app; one at a time. */
  private asking: symbol | null = null;
  /** Contents whose app link the operator declined; they ask again only after their next navigation. */
  private readonly declined = new Set<number>();
  private registry: readonly Command[] = systemRegistry(keySystemOf(process.platform));
  private cycleInput: { page: Page; release: string; cycleId: number } | null = null;
  private cycleSequence = 0;
  /** The page a blur cancelled a hold on, owed the keyboard once the window is back and the page shown. */
  private returnTo: Page | null = null;
  private reportedWindowFocus: boolean | null = null;
  private reportWindowFocus(focused: boolean): void {
    if (this.reportedWindowFocus === focused) return;
    this.reportedWindowFocus = focused;
    this.emit(this.local, { kind: "window-focus", focused });
    for (const source of this.sources.values()) this.emit(source, { kind: "window-focus", focused });
  }
  /**
   * Whether the window has had its `focus` event since its last `blur`. On
   * macOS that event is the window becoming main again, and Electron restores
   * the focus it stored on `blur` just before it; a window already key but not
   * yet back would hand that restore the keyboard a page was just given.
   */
  private windowBack = true;
  /** What took the keyboard in the window last on purpose: a page, a remote screen's view, or the shell (null). */
  private keyboard: Page | WebContents | null = null;

  setRegistry(registry: readonly Command[]): void { this.registry = registry; }

  /** Pays the keyboard owed to the page a blur cancelled a hold on, once the window is back and the page shown. */
  private giveBack(): void {
    const page = this.returnTo;
    if (!page) return;
    if (!page.visible || !this.windowBack) return;
    this.returnTo = null;
    this.focusPage(page);
    this.log.event("browser.window_return", { id: page.id, page_focused: page.view.webContents.isFocused() });
  }

  cdpIncarnation(scope: CdpScope): number | null { return this.cdpScopes.get(JSON.stringify([scope.workspace, scope.area_id])) ?? null; }

  /**
   * Only display pages enter CDP. The shell and popup windows are absent. A
   * page of the file partition is listed as `local` `checkout` and an outside
   * file's as `local` `outside`, which the gateway shows only to the purposes
   * that see them (`seesPage`; BROWSER_DISPLAYS.md, Comment mode and Agent
   * page commands).
   */
  cdpPages(): CdpPage[] {
    const result: CdpPage[] = [];
    for (const page of this.pages.values()) {
      const area_id = this.cdpAreas.get(page.key);
      const contents = page.view.webContents;
      // A remote screen's page is that computer's: no capability here reaches it.
      if (!area_id || contents.isDestroyed() || page.source.remote) continue;
      // An outside file reaches only the file capability of the pane that opened it, which hided alone names (B18).
      if (page.grant) result.push({ workspace: page.workspace, area_id, id: page.id, local: "outside", contents });
      // A checkout's page is listed only while it shows its checkout, so one a link moved to another file is driven by no one.
      else if (!page.fileRestricted) { if (this.showsCheckout(page, contents.getURL())) result.push({ workspace: page.workspace, area_id, id: page.id, local: "checkout", contents }); }
      else if (!isFileAddress(page.state.url) && !isFileAddress(page.route?.source_url ?? "") && !isFileAddress(contents.getURL())
        && !page.cdpFileFrame) {
        result.push({ workspace: page.workspace, area_id, id: page.id, contents });
      }
    }
    return result;
  }

  observeCdp(listener: (retirement?: CdpRetirement) => void): () => void {
    if (this.cdpChanged) throw new Error("Browser inventory already has a CDP owner");
    this.cdpChanged = listener;
    return () => { if (this.cdpChanged === listener) this.cdpChanged = null; };
  }

  /** Rect/visibility sync is frequent. Compare at most MAX_LIVE_VIEWS rows
   * and notify CDP only for membership, generation, area or metadata changes. */
  private publishCdp(retirement?: CdpRetirement): void {
    if (!this.cdpChanged) return;
    const next = this.cdpPages().map((page) => ({ key: viewKey(page.workspace, page.id), area: page.area_id, contents: page.contents.id, url: page.contents.getURL(), title: page.contents.getTitle() }));
    const changed = next.length !== this.cdpObserved.length || next.some((row, index) => {
      const previous = this.cdpObserved[index];
      return !previous || row.key !== previous.key || row.area !== previous.area || row.contents !== previous.contents || row.url !== previous.url || row.title !== previous.title;
    });
    // An ineligible document/frame invalidates the bound generation before
    // another protocol request can use it. Area moves are handled by scope.
    const excluded = this.cdpObserved.filter((previous) => !next.some((row) => row.key === previous.key)
      && [...this.pages.values()].some((page) => page.key === previous.key && page.contentsId === previous.contents));
    if (changed || retirement || this.cdpObservedRevision !== this.cdpScopesRevision) {
      this.cdpObserved = next;
      this.cdpObservedRevision = this.cdpScopesRevision;
      // The owner's session closes on a retirement and ends its observation with it, so each
      // notice goes to whoever still observes when it is sent.
      for (const row of excluded) this.cdpChanged?.({ contentsId: row.contents, reason: "replaced" });
      this.cdpChanged?.(retirement);
    }
  }

  /** A debugger lease is an actual host fact, independent of page load state. */
  cdpAttached(page: CdpPage, attached: boolean, purpose?: CdpPurpose): void {
    const owner = this.ownerOf(page.contents.id);
    if (attached && owner) owner.cdpRestricted = true;
    if (attached && owner && purpose === "file") owner.cdpFileHeld = true;
    if (attached) this.cdpLeases.set(page.contents.id, page);
    else this.cdpLeases.delete(page.contents.id);
    this.sendCdpAttached(page, attached);
  }

  private sendCdpAttached(page: { workspace: string; id: string }, attached: boolean): void {
    const contents = this.window?.webContents;
    if (contents && !contents.isDestroyed()) contents.send(BROWSER_EVENT_CHANNEL, { kind: "attached", workspace: page.workspace, id: page.id, attached });
  }

  private cancelCycle(): void {
    const held = this.cycleInput;
    this.cycleInput = null;
    if (held) this.emit(held.page.source, { kind: "cycle-cancel", cycleId: held.cycleId, workspace: held.page.workspace, id: held.page.id });
  }

  constructor(
    private readonly log: HostLog,
    /** Whether an IPC message came from the shell the daemon serves, not the status page or a page here. */
    private readonly trusted: (event: IpcMainEvent | IpcMainInvokeEvent) => boolean,
    private readonly resolve: (workspace: string, id: string, load: number) => Promise<ResolvedPage>,
    private readonly release: (workspace: string, id: string, load: number) => void,
    /** Shows a popup window the way the host shows its own, so a test run never activates the app. */
    private readonly present: (window: BrowserWindow, focus: boolean) => void,
    /** The host's translator in the language in effect now; asked for at each use, since the language can change while a page is open. */
    private readonly words: () => TFunction<"translation">,
    /** The outside files this window's operator granted. */
    private readonly externals: ExternalFiles,
    private readonly idle: IdleClock = { bound: IDLE_UNLOAD_MS, every: IDLE_CHECK_MS, now: Date.now },
  ) {
    ipcMain.on(BROWSER_CYCLE_END_CHANNEL, (event, cycleId: unknown) => {
      if (!this.trusted(event)) return this.log.event("browser.ipc_refused", { channel: "cycle_end" });
      if (this.cycleInput?.cycleId === cycleId) this.cycleInput = null;
    });
    ipcMain.on(BROWSER_SYNC_CHANNEL, (event, value: unknown) => {
      if (!this.trusted(event)) return this.log.event("browser.ipc_refused", { channel: "sync" });
      const sync = parseSync(value);
      if (!sync) return this.log.event("browser.sync_invalid", {});
      if (sync.node) this.local.node = sync.node;
      // UI owns this optional wire field. Its parser forwards it after the
      // shared contract lands; older shells have no replay epoch.
      const authority = sync as typeof sync & { attachment_epoch?: string; authorized_scopes?: { workspace: string; area_id: string; incarnation?: number }[] };
      this.sync(sync.workspace, sync.displays, sync.retained, sync.inventoried === true, authority.attachment_epoch, authority.authorized_scopes);
      if (sync.external_grants) this.keepGrants(new Set(sync.external_grants));
    });
    ipcMain.handle(BROWSER_CAPTURE_CHANNEL, async (event, value: unknown) => {
      if (!this.trusted(event)) return null;
      return this.capture(this.local, value);
    });
    ipcMain.on(BROWSER_COMMAND_CHANNEL, (event, value: unknown, name: unknown) => {
      if (!this.trusted(event)) return this.log.event("browser.ipc_refused", { channel: "command" });
      this.command(this.local, value, name);
    });
  }

  /**
   * A still of one of `source`'s pages for its shell. A covered page's still:
   * the shell asks while the page still shows, ahead of the sync that hides
   * it, and again for a page it keeps covered in the front Workspace whose
   * zoom moved under the zoom popover. A remote screen's page answers only
   * while it is drawn, one capture at a time and a few a second.
   */
  private async capture(source: Source, value: unknown): Promise<string | null> {
    const target = parseTarget(value);
    const page = target ? this.pages.get(pageKey(source, target.workspace, target.id)) : undefined;
    if (!page || (!page.visible && (source.remote || !page.placed))) {
      this.log.event("browser.capture_refused", { id: target?.id ?? null, reason: page ? "hidden" : "missing" });
      return null;
    }
    if (source.remote) {
      const now = this.idle.now();
      source.captured = recent(source.captured, now, 1_000);
      if (source.capturing || source.captured.length >= REMOTE_BOUNDS.capturesPerSecond) {
        this.log.event("browser.capture_refused", { id: page.id, reason: "remote_bound" });
        return null;
      }
      source.captured.push(now);
      source.capturing = true;
    }
    try {
      const image = await page.view.webContents.capturePage();
      if (!image.isEmpty()) return image.toDataURL();
      this.log.event("browser.capture_empty", { id: page.id });
      return null;
    } catch (error) {
      this.log.event("browser.capture_failed", { id: page.id, detail: String(error) });
      return null;
    } finally {
      source.capturing = false;
    }
  }

  /** A toolbar command for one of `source`'s pages; a remote screen's at most `commandsPerSecond`. */
  private command(source: Source, value: unknown, name: unknown): void {
    const target = parseTarget(value);
    const command = parseCommand(name);
    if (!target || !command) return;
    const page = this.pages.get(pageKey(source, target.workspace, target.id));
    if (!page && !(source.remote && command === "reload")) return;
    if (source.remote) {
      const now = this.idle.now();
      source.commanded = recent(source.commanded, now, 1_000);
      if (source.commanded.length >= REMOTE_BOUNDS.commandsPerSecond) {
        if (!source.commandsRefused) this.log.event("browser.command_refused", { reason: "remote_bound" });
        source.commandsRefused = true;
        return;
      }
      source.commandsRefused = false;
      source.commanded.push(now);
      // A slot that failed before its page was made (its proxy, its storage,
      // the page bound) has its Reload apply the screen's last sync again.
      if (!page) return this.applyRemote(source);
    }
    if (!page) return;
    const contents = page.view.webContents;
    const zoom = SHELL_ZOOM_COMMANDS[command];
    if (zoom) this.zoom(page, zoom);
    else if (command === "focus") {
      // A remote screen's page takes the keyboard only from inside that screen.
      const inside = !source.remote || source.remote.focused() || this.focused()?.source === source;
      if (page.visible && this.window?.isFocused() && inside) this.focusPage(page);
    } else if (command === "back" && contents.navigationHistory.canGoBack()) contents.navigationHistory.goBack();
    else if (command === "forward" && contents.navigationHistory.canGoForward()) contents.navigationHistory.goForward();
    else if (command === "stop") contents.stop();
    else if (command === "reload") {
      // A page that failed to load has nothing to reload; its address loads
      // again. A remote screen's page asks that computer's core again for the
      // display's address as the screen last named it, not a link that
      // failed in it nor the address its first route answered; a file page
      // of that computer always does, since its route may have ended with
      // the screen's view.
      const remoteFile = source.remote && page.route && isFileAddress(page.route.source_url);
      if (page.state.failure || remoteFile) this.load(page, source.remote ? (page.address ?? page.state.url) : page.state.url);
      else contents.reload();
    }
  }

  // --- another computer's screen ---------------------------------------------

  /**
   * A remote screen whose page may place that computer's browser displays
   * here. One per machine and key at a time: a screen made again takes the
   * place of the one before, whose pages close with it.
   */
  addRemote(remote: RemoteSource): void {
    const id = remoteId(remote);
    const earlier = this.sources.get(id);
    if (earlier?.remote) this.removeRemote(earlier.remote);
    const source = newSource(id, remote);
    this.sources.set(id, source);
  }

  /** The screen is gone (trimmed, another key, unpaired, every screen closed): its pages and popups close with it. */
  removeRemote(remote: RemoteSource): void {
    const source = this.sourceOf(remote);
    if (!source) return;
    for (const page of [...this.pages.values()]) if (page.source === source) this.destroy(page, "closed");
    this.forgetSource(source);
    this.sources.delete(source.id);
  }

  /**
   * Every page of the machine `device` under `key` closes now, its popups
   * with it: its storage is about to be cleared, or its pair ended. The
   * screen, which may still show, keeps its place and its last sync, and
   * makes its pages again once its storage may hold them (`remoteRetry`).
   */
  closeRemotePages(device: string, key: string): void {
    const source = this.sources.get(remoteId({ device, key }));
    if (!source) return;
    for (const page of [...this.pages.values()]) if (page.source === source) this.destroy(page, "closed");
  }

  /**
   * Every page of a remote screen on `partition` closes, its popups with it:
   * that partition's storage is about to be cleared. A page of the same
   * machine on another pairing's partition stays.
   */
  closeRemotePartition(partition: string): void {
    for (const page of [...this.pages.values()]) if (page.source.remote && page.partition === partition) this.destroy(page, "closed");
  }

  /** The machine's storage was cleared: its screen places its pages again, empty. */
  remoteRetry(device: string, key: string): void {
    const source = this.sources.get(remoteId({ device, key }));
    if (source) this.applyRemote(source);
  }

  /** The remote source whose pages keep their logins, or load their files, in `partition` now; none for an ended pairing's. */
  private remoteSourceOf(partition: string): Source | null {
    for (const source of this.sources.values()) {
      if (source.remote && (source.remote.partition() === partition || source.remote.filePartition() === partition)) return source;
    }
    return null;
  }

  private sourceOf(remote: RemoteSource): Source | null {
    const source = this.sources.get(remoteId(remote));
    return source?.remote === remote ? source : null;
  }

  private forgetSource(source: Source): void {
    if (source.syncTimer) clearTimeout(source.syncTimer);
    if (source.createTimer) clearTimeout(source.createTimer);
    source.syncTimer = source.createTimer = null;
    source.latest = null;
  }

  /** A sync from the screen's page, checked as this window's shell's is with that screen's bounds, applied while it shows. */
  remoteSync(remote: RemoteSource, value: unknown): void {
    const source = this.sourceOf(remote);
    if (!source) return;
    const sync = parseRemoteSync(value);
    if (!sync) return this.log.event("browser.sync_invalid", { source: "remote" });
    if (sync.node) source.node = sync.node;
    source.latest = sync;
    this.applyRemote(source);
  }

  remoteCapture(remote: RemoteSource, value: unknown): Promise<string | null> {
    const source = this.sourceOf(remote);
    return source ? this.capture(source, value) : Promise.resolve(null);
  }

  remoteCommand(remote: RemoteSource, value: unknown, name: unknown): void {
    const source = this.sourceOf(remote);
    if (source) this.command(source, value, name);
  }

  remoteCycleEnd(remote: RemoteSource, cycleId: unknown): void {
    const source = this.sourceOf(remote);
    if (source && this.cycleInput?.page.source === source && this.cycleInput.cycleId === cycleId) this.cycleInput = null;
  }

  /** A key the screen's own view takes while a hold started in one of its pages runs: the hold's release or Escape ends it there. */
  remoteInput(remote: RemoteSource, event: { preventDefault(): void }, input: Input): void {
    const source = this.sourceOf(remote);
    if (source && this.cycleInput?.page.source === source) this.routeCycleInput(event, input);
  }

  /** The screen shows again: its pages take their places from its last sync, and its popups show. */
  remoteShown(remote: RemoteSource): void {
    const source = this.sourceOf(remote);
    if (!source) return;
    // Shown again after it hid; a show that only moves the tile re-shows nothing.
    if (source.hidden) for (const popup of this.popups.values()) if (popup.owner.source === source && !popup.window.isDestroyed()) popup.window.showInactive();
    source.hidden = false;
    this.applyRemote(source);
  }

  /** The screen hid or failed: its pages and popups hide and stay, and its syncs wait until it shows. */
  remoteHidden(remote: RemoteSource): void {
    const source = this.sourceOf(remote);
    if (!source) return;
    const now = this.idle.now();
    source.hidden = true;
    for (const page of this.pages.values()) {
      if (page.source !== source) continue;
      if (page.placed || page.visible) page.idleSince = now;
      page.placed = false;
      this.show(page, false);
    }
    for (const popup of this.popups.values()) if (popup.owner.source === source && !popup.window.isDestroyed()) popup.window.hide();
  }

  /** The screen's page starts a new document: nothing is placed until it says where. */
  remoteReset(remote: RemoteSource): void {
    const source = this.sourceOf(remote);
    if (!source) return;
    this.remoteHidden(remote);
    source.latest = null;
    source.newestLoad = null;
  }

  /** Another owner added a view to the window; a page added before it is raised when next placed. */
  viewAdded(): void {
    this.viewsAdded++;
  }

  /** A remote screen's view took the keyboard: a page's navigation gives it back there. */
  viewFocused(contents: WebContents): void {
    this.keyboard = contents;
  }

  /**
   * A remote screen's last sync, applied while it shows: at most one every
   * `syncEveryMs`, nothing made before its pages' proxy is set or while its
   * storage is being cleared, at most `REMOTE_BOUNDS.pages` live pages and
   * `REMOTE_BOUNDS.creations` made in `creationWindowMs`, and every page cut
   * to the screen's tile. No page is loaded off screen.
   */
  private applyRemote(source: Source): void {
    const remote = source.remote;
    const window = this.window;
    const sync = source.latest;
    if (!remote || !window || window.isDestroyed() || !sync) return;
    const area = remote.area();
    if (!area) return;
    const now = this.idle.now();
    const wait = source.syncedAt + REMOTE_BOUNDS.syncEveryMs - now;
    if (wait > 0) {
      source.syncTimer ??= setTimeout(() => {
        source.syncTimer = null;
        this.applyRemote(source);
      }, wait);
      return;
    }
    source.syncedAt = now;
    const workspace = sync.workspace ?? "";
    const listed = new Set(sync.retained.map((row) => pageKey(source, row.workspace, row.id)));
    const placed = new Set(sync.displays.filter((display) => display.rect).map((display) => pageKey(source, workspace, display.id)));
    for (const page of [...this.pages.values()]) {
      if (page.source !== source) continue;
      if (!listed.has(page.key)) this.destroy(page, "closed");
      else if (page.workspace !== workspace || !placed.has(page.key)) {
        if (page.placed || page.visible) page.idleSince = now;
        page.placed = false;
        this.show(page, false);
      }
    }
    if (!remote.ready()) {
      // The proxy goes first; nothing of the screen's is made before it is set.
      if (source.egressAsked) return;
      source.egressAsked = true;
      void remote.ensure().then((state) => {
        source.egressAsked = false;
        if (this.sources.get(source.id) !== source) return;
        if (remote.ready()) this.applyRemote(source);
        else if (source.latest) this.failSlots(source, source.latest, egressWords(state));
      });
      return;
    }
    // The storage is the pairing's, which the proxy's answer named.
    const storage = remote.storage();
    if (storage === "waiting") return;
    if (storage !== "live") return this.failSlots(source, sync, storage === "full" ? "remoteStorageFull" : storage === "unrecorded" ? "remoteStorageUnrecorded" : "remoteStorageClearing");
    source.created = recent(source.created, now, REMOTE_BOUNDS.creationWindowMs);
    for (const display of sync.displays) {
      if (!display.rect) continue;
      const key = pageKey(source, workspace, display.id);
      let page = this.pages.get(key);
      if (!page) {
        if (source.created.length >= REMOTE_BOUNDS.creations) {
          // It loads once the window has room again; no hidden page closes for it before then.
          source.createTimer ??= setTimeout(() => {
            source.createTimer = null;
            this.applyRemote(source);
          }, REMOTE_BOUNDS.creationWindowMs - (now - source.created[0]!));
          continue;
        }
        this.evictOverCap(source);
        if ([...this.pages.values()].filter((live) => live.source === source).length >= REMOTE_BOUNDS.pages && !this.evictHidden(source, placed)) {
          this.failSlot(source, workspace, display, "remotePageLimit");
          continue;
        }
        source.created.push(now);
      }
      page = this.refresh(source, window, workspace, display).page;
      const bounds = placeIn(display.rect, area.zoom, area.bounds);
      page.offScreen = false;
      page.placed = true;
      page.device = display.device ?? null;
      page.shownAt = now;
      page.idleSince = now;
      if (!bounds) {
        this.show(page, false);
        continue;
      }
      // Above the screen's view, which may have been added after it.
      if (page.addedAt < this.viewsAdded) {
        window.contentView.addChildView(page.view);
        page.addedAt = this.viewsAdded;
      }
      page.view.setBounds(bounds);
      page.bounds = bounds;
      this.emulate(page);
      this.show(page, display.visible);
    }
    this.evictOverCap(source);
  }

  /** Every display a sync places says why it has no page. */
  private failSlots(source: Source, sync: BrowserSync, words: string): void {
    for (const display of sync.displays) if (display.rect) this.failSlot(source, sync.workspace ?? "", display, words);
  }

  /** A display of a remote screen that gets no page: its slot says why, never a blank. */
  private failSlot(source: Source, workspace: string, display: BrowserPlacement, words: string): void {
    if (this.pages.has(pageKey(source, workspace, display.id))) return;
    const failure = this.words()(`native.browser.${words}`);
    this.emit(source, { kind: "state", workspace, id: display.id, load: display.load, state: { url: display.url, title: "", loading: false, canGoBack: false, canGoForward: false, failure, zoom: 1 } });
  }

  /** The window the pages are drawn in. A new window starts with no pages. */
  attach(window: BrowserWindow): void {
    this.window = window;
    // One check while the window lives, the only work an idle screen adds (B12).
    if (this.idleTimer) clearInterval(this.idleTimer);
    this.idleTimer = setInterval(() => this.unloadIdle(), this.idle.every);
    // Once a native page starts a hold, its shell input uses the same IPC
    // route. Even a quick release cannot overtake the forwarded first chord.
    window.webContents.on("before-input-event", (event, input) => this.routeCycleInput(event, input));
    // A shell that loads again, or moves to another daemon, has not placed
    // anything yet: its pages hide until it says where they go.
    window.webContents.on("did-start-navigation", (details) => {
      if (!details.isMainFrame || details.isSameDocument) return;
      this.hideAll(this.local);
      this.local.newestLoad = null;
    });
    window.on("blur", () => {
      this.reportWindowFocus(false);
      this.windowBack = false;
      const held = this.cycleInput;
      this.cancelCycle();
      // The hold moved the native responder to the shell. When the window
      // comes back, the page that started it takes the keyboard again, as
      // it would after Escape, once it is shown: the overlay that covered it
      // goes away through the shell's next sync, which can land after the window returns.
      if (held) this.returnTo = held.page;
      this.log.event("browser.window_blur", { held: held !== null, id: held?.page.id ?? null, owed: this.returnTo !== null });
    });
    window.on("focus", () => {
      this.reportWindowFocus(true);
      this.windowBack = true;
      this.log.event("browser.window_focus", { owed: this.returnTo !== null, page_visible: this.returnTo?.visible ?? null });
      this.giveBack();
    });
    // Where the keyboard goes after a window's return: the order of these, the
    // pages' focus, blur and visibility after `browser.window_return`, says what took it.
    window.webContents.on("focus", () => {
      this.keyboard = null;
      this.log.event("browser.shell_focus", { window_focused: window.isFocused(), owed: this.returnTo !== null });
    });
    // macOS reports the window hidden while nothing of it is drawn (covered,
    // or ordered out), and the shell then draws no frame, so it sends no sync.
    window.on("show", () => this.log.event("browser.window_visible", { visible: true, owed: this.returnTo !== null }));
    window.on("hide", () => this.log.event("browser.window_visible", { visible: false, owed: this.returnTo !== null }));
    window.on("closed", () => {
      this.cancelCycle();
      for (const page of [...this.pages.values()]) this.destroy(page, "window_closed");
      if (this.window === window) {
        this.window = null;
        this.unloaded.clear();
        this.local.newestLoad = null;
        for (const source of this.sources.values()) this.forgetSource(source);
        this.sources.clear();
        if (this.idleTimer) clearInterval(this.idleTimer);
        this.idleTimer = null;
      }
    });
  }

  /** Whether a page holds the keyboard, so an app-menu command gives it back to the shell first. */
  hasFocus(): boolean {
    return this.focused() !== undefined;
  }

  /**
   * What an app-menu command gives the keyboard back to while a page holds
   * it: the shell that placed the page, this window's or a remote screen's
   * view, which then takes the command; null while no page holds it.
   */
  focusHolder(): WebContents | null {
    const page = this.focused();
    if (!page) return null;
    return page.source.remote ? page.source.remote.contents() : (this.window?.webContents ?? null);
  }

  /**
   * A text-size command while a page holds the keyboard zooms that page and
   * leaves it the keyboard; any other command, or no focused page, is not
   * handled here. Routed by command, so a rebound chord zooms too. A page
   * shown at a device size takes the command and zooms nothing, so neither
   * the page nor a terminal's text moves (PRD browser-width-zoom B13).
   */
  zoomFocused(command: CommandId): boolean {
    const zoom = PAGE_ZOOM_COMMANDS[command];
    const page = zoom && this.focused();
    if (!zoom || !page) return false;
    this.zoom(page, zoom);
    return true;
  }

  private focused(): Page | undefined {
    for (const page of this.pages.values()) if (page.visible && page.view.webContents.isFocused()) return page;
    return undefined;
  }

  /** A page at a device size keeps its zoom: its viewport is the device's width whatever the zoom. */
  private zoom(page: Page, zoom: PageZoom): void {
    if (page.device) return;
    zoomContents(page.view.webContents, zoom);
    this.zoomMoved(page.partition);
  }

  /**
   * Chromium keeps one zoom per host of a session, so a zoom anywhere moves
   * every page of that host, in any Workspace, and no event says so: each
   * page of the session reports its zoom again when it moved, and one at a
   * device size has its viewport fitted again (PRD browser-width-zoom B9).
   * At most `MAX_LIVE_VIEWS` pages are read, and only on a zoom.
   */
  private zoomMoved(partition: string): void {
    for (const page of this.pages.values()) {
      if (page.partition !== partition || page.view.webContents.isDestroyed()) continue;
      const zoom = page.view.webContents.getZoomFactor();
      if (page.state.zoom !== zoom) this.update(page, { zoom });
      this.emulate(page);
    }
  }

  /**
   * Lays the page out at its device size, or at its slot's again (PRD
   * browser-width-zoom D-12); applied only when what it asks for changed, and
   * again after every committed navigation, since a new document, or a new
   * renderer, starts without it.
   */
  private emulate(page: Page, force = false): void {
    const contents = page.view.webContents;
    if (contents.isDestroyed()) return;
    // Emulating a page no slot has shown before it has a document stalls the
    // main process, at a device size too; its commit applies the layout.
    if (page.offScreen && !contents.getURL()) return;
    const params = !page.bounds ? null
      : page.device ? deviceEmulation(page.device, page.bounds, contents.getZoomFactor())
      : page.offScreen ? offScreenEmulation(page.bounds) : null;
    const key = params ? JSON.stringify(params) : null;
    if (!force && key === page.emulated) return;
    if (params) contents.enableDeviceEmulation(params);
    else if (page.emulated !== null) contents.disableDeviceEmulation();
    page.emulated = key;
  }

  /**
   * An app-menu command while a popup window holds the keyboard: Close closes
   * the popup, text size zooms it, and nothing else reaches the shell behind
   * it, so ⌘W in a sign-in window never closes one of the operator's views.
   */
  popupCommand(command: CommandId): boolean {
    const focused = BrowserWindow.getFocusedWindow();
    const popup = focused ? [...this.popups.values()].find((row) => row.window === focused) : undefined;
    if (!popup) return false;
    const zoom = PAGE_ZOOM_COMMANDS[command];
    if (command === "close_tab" || command === "close_pane") popup.window.close();
    else if (zoom) {
      zoomContents(popup.window.webContents, zoom);
      this.zoomMoved(popup.owner.partition);
    }
    return true;
  }

  private sync(workspace: string | null, displays: BrowserPlacement[], retained: BrowserSync["retained"], inventoried: boolean, attachmentEpoch?: string, authorizedScopes?: { workspace: string; area_id: string; incarnation?: number }[]): void {
    const window = this.window;
    if (!window) return;
    this.reportWindowFocus(window.isFocused());
    // The shared parser bounds this list. Missing authority grants nothing;
    // retained display rows can never restore a revoked checkout/area.
    const rows = authorizedScopes && authorizedScopes.length <= MAX_AUTHORIZED_SCOPES ? authorizedScopes : [];
    const scopes = new Map(rows.flatMap((scope) => Number.isSafeInteger(scope.incarnation) && scope.incarnation! >= 0
      ? [[JSON.stringify([scope.workspace, scope.area_id]), scope.incarnation!] as const] : []));
    if (scopes.size !== this.cdpScopes.size || [...scopes].some(([scope, incarnation]) => this.cdpScopes.get(scope) !== incarnation)) {
      this.cdpScopes = scopes;
      this.cdpScopesRevision++;
    }
    // A missing area from an older shell grants no capability. parseSync's
    // current contract requires it and matches placement to retained rows.
    this.cdpAreas = new Map(retained.flatMap((row) => row.area_id ? [[viewKey(row.workspace, row.id), row.area_id] as const] : []));
    const listed = new Set(retained.map((row) => viewKey(row.workspace, row.id)));
    for (const key of this.unloaded.keys()) if (!listed.has(key)) this.unloaded.delete(key);
    const placed = new Set(displays.map((display) => viewKey(workspace ?? "", display.id)));
    for (const page of [...this.pages.values()]) {
      if (page.source !== this.local) continue;
      if (!listed.has(page.key)) this.destroy(page, "closed");
      else if (page.workspace !== workspace || !placed.has(page.key)) {
        // A retained page outside the front presentation stays alive and hidden.
        if (this.returnTo === page) this.returnTo = null;
        if (page.placed || page.visible) page.idleSince = this.idle.now();
        page.placed = false;
        this.show(page, false);
      }
    }
    const now = this.idle.now();
    const zoom = window.webContents.getZoomFactor();
    const onSlot = new Set<string>();
    for (const display of displays) {
      const key = viewKey(workspace ?? "", display.id);
      let page = this.pages.get(key);
      if (!display.rect) {
        if (page) {
          if (page.placed || page.visible) page.idleSince = now;
          page.placed = false;
          this.show(page, false);
        }
        continue;
      }
      onSlot.add(key);
      page = this.refresh(this.local, window, workspace ?? "", display).page;
      const bounds = toBounds(display.rect, zoom);
      page.offScreen = false;
      page.view.setBounds(bounds);
      page.placed = true;
      page.device = display.device ?? null;
      page.bounds = bounds;
      this.emulate(page);
      this.show(page, display.visible);
      page.shownAt = now;
      page.idleSince = now;
      this.unloaded.delete(key);
    }
    // Only a sync that carries the core's inventory asks or moves the baseline.
    const baseline = inventoried ? this.local.newestLoad : null;
    for (const row of retained) if (!onSlot.has(viewKey(row.workspace, row.id))) this.offScreen(window, row, now, baseline);
    if (inventoried) this.local.newestLoad = retained.reduce((newest, row) => Math.max(newest, row.load ?? 0), this.local.newestLoad ?? 0);
    this.evictOverCap(this.local);
    this.publishCdp();
    if (attachmentEpoch && /^[A-Za-z0-9-]{1,64}$/.test(attachmentEpoch) && attachmentEpoch !== this.attachmentEpoch) {
      this.attachmentEpoch = attachmentEpoch;
      for (const page of this.pages.values()) if (page.source === this.local) this.sendCdpAttached(page, this.cdpLeases.has(page.contentsId));
    }
  }

  /**
   * A retained page no slot on screen shows: on no desk, on the desk of an
   * Agent tab or a Workspace not in front, or under a folded desk (PRD
   * lane-desk-pair-view B32, D-37). A web page whose stamp is newer than
   * `baseline`, the newest this shell had sent before, is one the core asked
   * to load: it is loaded here hidden, so an agent's `hide browser` commands
   * and `--wait` find it loaded and the desk later shows the same page, and
   * it counts toward `MAX_LIVE_VIEWS` as just seen. A live page loads a newer
   * stamp where it is. One the core asks nothing of follows its address and
   * stamp, for `cdpRevive`. An outside file names no address: it loads when
   * shown, or when the agent that opened it drives it (`cdpRevive`, by its grant).
   */
  private offScreen(window: BrowserWindow, row: BrowserSync["retained"][number], now: number, baseline: number | null): void {
    if (row.url === undefined || row.load === undefined) return;
    const key = viewKey(row.workspace, row.id);
    const live = this.pages.get(key);
    const record = this.unloaded.get(key);
    if (!live && (baseline === null || row.load <= baseline)) {
      // A web page and a checkout's file page enter CDP, so either is loaded again for an agent's connection.
      if (record) Object.assign(record, { url: row.url, load: row.load, area_id: row.area_id });
      else this.unloaded.set(key, { workspace: row.workspace, id: row.id, area_id: row.area_id, url: row.url, load: row.load, device: null, bounds: null });
      return;
    }
    const { page, changed } = this.refresh(this.local, window, row.workspace, { id: row.id, area_id: row.area_id, url: row.url, load: row.load, rect: null, visible: false });
    if (!changed) return;
    if (page !== live) {
      // A new page, or one in another partition, has had no slot show it.
      page.device = live?.device ?? record?.device ?? null;
      page.bounds = live?.bounds ?? record?.bounds ?? offScreenBounds(window.getContentBounds());
      page.offScreen = true;
      page.view.setBounds(page.bounds);
      this.emulate(page);
      this.unloaded.delete(key);
    }
    page.shownAt = page.idleSince = now;
    this.log.event("browser.loaded_off_screen", { id: page.id, live: this.pages.size });
  }

  /**
   * The live page of a display, on a slot or off screen, as the core last
   * asked for it: created when there is none, made again when its address
   * needs another partition, or loaded again when its stamp is newer than the
   * one it loaded. `changed` is whether any of these happened.
   */
  private refresh(source: Source, window: BrowserWindow, workspace: string, display: BrowserPlacement): { page: Page; changed: boolean } {
    const page = this.pages.get(pageKey(source, workspace, display.id));
    if (!page) return { page: this.create(source, window, workspace, display), changed: true };
    if (page.partition !== this.partitionOf(source, workspace, display)) {
      this.destroy(page, "replaced");
      return { page: this.create(source, window, workspace, display), changed: true };
    }
    if (source.remote) page.address = display.url;
    if (display.load <= page.applied) return { page, changed: false };
    page.applied = display.load;
    this.load(page, this.addressOf(display));
    return { page, changed: true };
  }

  /**
   * An outside file's own session, kept in memory and shared with no other
   * page (B4); every other page's from its address (`browserPartition`).
   */
  private partitionOf(source: Source, workspace: string, display: BrowserPlacement): string {
    // A remote screen's web pages keep that machine's login storage; its
    // file pages load apart from it, so a checkout's HTML reads no login.
    // None is made before its proxy's answer named one (`ready`); were one
    // asked for, its session belongs to no source and refuses every request.
    if (source.remote) {
      return isFileAddress(display.url)
        ? (source.remote.filePartition() ?? `${REMOTE_FILE_PARTITION}unproxied`)
        : (source.remote.partition() ?? `${REMOTE_PAGE_PARTITION}unproxied`);
    }
    return display.grant ? (this.externals.partition(display.grant) ?? GONE_GRANT_PARTITION) : browserPartition(workspace, display.url, source.node);
  }

  /** What a display loads: its address, or the file its grant names, which only this process knows. */
  private addressOf(display: BrowserPlacement): string {
    return display.grant ? (this.externals.pageUrl(display.grant) ?? "") : display.url;
  }

  /**
   * Lets go of every grant no tab stands on (`ExternalFiles.keep`). A page
   * of one is closed with it and its session's storage cleared, so the page
   * reads nothing more from the folder (B14).
   */
  private keepGrants(named: ReadonlySet<string>): void {
    const gone = this.externals.keep(named);
    if (gone.length === 0) return;
    const ids = new Set(gone.map((released) => released.id));
    for (const page of [...this.pages.values()]) if (page.grant && ids.has(page.grant)) this.destroy(page, "closed");
    for (const { id, slot } of gone) {
      if (slot === null) continue;
      const partition = externalPartition(slot);
      if (!this.configuredSessions.has(partition)) {
        this.externals.slotCleared(slot);
        continue;
      }
      // The session keeps its handlers, which name no grant, so a page reusing
      // the slot adds none; a slot whose storage or cache could not be
      // cleared is never handed out again, so no later page reads it.
      const pageSession = session.fromPartition(partition);
      void Promise.all([pageSession.clearStorageData(), pageSession.clearCache()]).then(
        () => this.externals.slotCleared(slot),
        (error: unknown) => this.log.event("external.storage_clear_failed", { grant: id, reason: errorCode(error) }),
      );
    }
  }

  private create(source: Source, window: BrowserWindow, workspace: string, display: BrowserPlacement): Page {
    const page = this.construct(source, window, workspace, display, this.partitionOf(source, workspace, display));
    this.load(page, this.addressOf(display));
    return page;
  }

  /** A page for `display` in its own new contents, added to the window hidden and not loaded. */
  private construct(source: Source, window: BrowserWindow, workspace: string, display: Pick<BrowserPlacement, "id" | "url" | "load" | "grant">, partition: string): Page {
    // A remote screen's page shows no sheet over this window and downloads
    // no dictionary; its contents send no UDP of their own around the proxy.
    const remote = source.remote ? { disableDialogs: true, spellcheck: false } : {};
    const view = new WebContentsView({
      webPreferences: { session: this.sessionFor(partition), sandbox: true, contextIsolation: true, nodeIntegration: false, webSecurity: true, ...remote },
    });
    if (source.remote) view.webContents.setWebRTCIPHandlingPolicy("disable_non_proxied_udp");
    const page: Page = {
      key: pageKey(source, workspace, display.id),
      source,
      addedAt: this.viewsAdded,
      workspace,
      id: display.id,
      view,
      contentsId: view.webContents.id,
      applied: display.load,
      visible: false,
      placed: false,
      shownAt: 0,
      idleSince: this.idle.now(),
      playing: false,
      downloads: 0,
      settled: new Set(),
      state: { url: display.url, title: "", loading: true, canGoBack: false, canGoForward: false, failure: null, zoom: 1 },
      device: null,
      bounds: null,
      emulated: null,
      report: null,
      route: null,
      loading: null,
      partition,
      fileRestricted: !partition.startsWith("persist:hide-browser-file-") && !display.grant,
      cdpRestricted: false,
      cdpDownloadRefused: false,
      cdpFileFrame: false,
      fileRefusals: new Set(),
      remoteRefusals: new Set(),
      ...(source.remote ? { address: display.url } : {}),
      grab: null,
      hostVerdicts: new Map(),
      ...(display.grant ? { grant: display.grant } : {}),
    };
    view.setVisible(false);
    window.contentView.addChildView(view);
    this.pages.set(page.key, page);
    this.watch(page);
    this.log.event("browser.view_created", { live: this.pages.size });
    return page;
  }

  /**
   * A page that ended its own contents (`window.close()`, which Chromium
   * allows a page that opened with one history entry) leaves its view with
   * none. It is closed as the core's close would close it and a fresh, unloaded
   * page takes its place on the same slot, showing why with the toolbar's
   * reload; loading it again on its own would loop on a page that closes
   * itself as it loads.
   */
  private closedItself(page: Page): void {
    this.log.event("browser.page_closed_itself", { id: page.id, contents: page.contentsId });
    this.destroy(page, "closed");
    const window = this.window;
    if (!window || window.isDestroyed()) return;
    const fresh = this.construct(page.source, window, page.workspace, { id: page.id, url: page.state.url, load: page.applied, grant: page.grant }, page.partition);
    Object.assign(fresh, { placed: page.placed, shownAt: page.shownAt, idleSince: page.idleSince, device: page.device, bounds: page.bounds, address: page.address });
    if (fresh.bounds) fresh.view.setBounds(fresh.bounds);
    this.emulate(fresh);
    this.show(fresh, page.visible);
    this.update(fresh, { url: page.state.url, title: page.state.title, loading: false, failure: this.words()("native.browser.closedItself") });
  }

  private sessionFor(partition: string): Session {
    const pageSession = session.fromPartition(partition);
    if (this.configuredSessions.has(partition)) return pageSession;
    this.configuredSessions.add(partition);
    // A remote screen's login or file page storage, whichever screen of its machine uses it now.
    const remote = partition.startsWith(REMOTE_PAGE_PARTITION) || partition.startsWith(REMOTE_FILE_PARTITION);
    pageSession.setPermissionRequestHandler((contents, permission, callback, details) => {
      // A link to another app that no navigation guard saw (a frame's, say)
      // reaches Chromium's external protocol handler, which asks here. The
      // question is hide's own, so Chromium's opener never runs.
      if (permission !== "openExternal") return callback(PAGE_PERMISSIONS.has(permission));
      callback(false);
      const page = this.ownerOf(contents.id);
      const url = "externalURL" in details ? details.externalURL : undefined;
      if (page && url) this.openInApp(page, contents, url, details.requestingUrl);
      else this.log.event("browser.app_link_refused", { reason: "no_page" });
    });
    pageSession.setPermissionCheckHandler((_contents, permission) => PAGE_PERMISSIONS.has(permission));
    pageSession.on("will-download", (event, item, contents) => {
      const page = this.ownerOf(contents.id);
      // A file a remote screen's page downloads would be that computer's,
      // which this computer has no place for: refused, and logged once.
      if (remote) {
        event.preventDefault();
        if (page && !page.downloadRefused) {
          page.downloadRefused = true;
          this.log.event("browser.download_refused", { reason: "remote_screen", display_id: page.id });
        }
        return;
      }
      if (page?.cdpRestricted) {
        event.preventDefault();
        if (!page.cdpDownloadRefused) {
          page.cdpDownloadRefused = true;
          this.log.event("browser.download_refused", { reason: "cdp_filesystem_boundary", display_id: page.id });
        }
      } else {
        this.log.event("browser.download", { mime: item.getMimeType() });
        // A download in progress holds its page alive past the idle bound (B3).
        if (page) {
          page.downloads++;
          item.once("done", () => { page.downloads--; });
        }
      }
    });
    pageSession.webRequest.onBeforeRequest({ urls: ["<all_urls>"] }, (details, callback) => {
      const page = this.ownerOf(details.webContentsId);
      if (remote) return callback(this.remoteRequest(partition, page, details));
      if (page?.fileRestricted && isFileAddress(details.url)) {
        this.fileRefused(page, "request", details.resourceType);
        callback({ cancel: true });
        return;
      }
      if (page?.route && !routed(page.route) && isFileAddress(page.route.source_url)) {
        this.localPageRequest(pageSession, page, details, callback);
        return;
      }
      const outcome = !page
        ? partition === "persist:hide-browser-web" ? remoteRequest({ source_url: details.url, url: details.url }, details.url) : { cancel: true }
        : !page.route ? { cancel: true }
        : this.local.node !== null && page.workspace.startsWith(`${this.local.node}\u0000`) ? {}
        : remoteRequest(page.route, details.url);
      callback(outcome);
    });
    return pageSession;
  }

  /**
   * A request of a remote screen's page, which its session's proxy carries
   * to that computer: none before the proxy is set or once its source is
   * gone, none in a file page's session that no page of it owns, no file of
   * this computer, nothing but its own route from a page that loads from
   * one (a file page of that computer: its own route's origin alone), nothing on that computer's loopback from a page its
   * route did not call its own, and nothing on a local network from a
   * public document (`localNetworkReach`). Each refusal is logged once per
   * page and reason, with its scheme alone.
   */
  private remoteRequest(partition: string, page: Page | undefined, details: Electron.OnBeforeRequestListenerDetails): { cancel: boolean } {
    const source = this.remoteSourceOf(partition);
    const mainFrame = details.resourceType === "mainFrame";
    let host: string | null = null;
    try { host = new URL(details.url).hostname; } catch { /* refused below */ }
    const route = page?.route && routed(page.route) ? page.route : null;
    const onRoute = route ? remoteRequest(route, details.url) : null;
    // A file page's session answers only its pages' own routes: a request
    // no page of it owns (a worker's, an unknown contents') has none.
    const filePage = partition.startsWith(REMOTE_FILE_PARTITION);
    const reason = !source?.remote?.ready() || (page && page.source !== source) ? "not_ready"
      : filePage && !route ? "no_page"
      : isFileAddress(details.url) || host === null ? "file"
      : onRoute ? (onRoute.cancel || onRoute.redirectURL ? "off_route" : null)
      : page?.own === false && isLoopbackHost(host) ? "not_own"
      : !localNetworkReach(details.url, mainFrame ? null : this.topOf(details.webContentsId), mainFrame) ? "local_network"
      : null;
    if (reason === null) return { cancel: false };
    const seen = page?.remoteRefusals ?? source?.refusals;
    if (seen && !seen.has(reason)) {
      seen.add(reason);
      this.log.event("browser.request_refused", { reason: `remote_${reason}`, protocol: protocolOf(details.url), ...(page ? { display_id: page.id } : {}), resource_type: details.resourceType });
    }
    return { cancel: true };
  }

  /** The address of the document a request comes from; null when it has none (a service worker). */
  private topOf(contentsId: number | undefined): string | null {
    if (contentsId === undefined) return null;
    const contents = allContents.fromId(contentsId);
    return contents && !contents.isDestroyed() ? contents.getURL() || null : null;
  }

  /**
   * A local HTML page's request, held to what a browser allows a local page
   * (`localPageRequest`). A public name is resolved before it is allowed, and
   * one that answers with a private address is refused; the answer is kept
   * for the generation, so a name that changes its answer later is not seen.
   */
  private localPageRequest(pageSession: Session, page: Page, details: Electron.OnBeforeRequestListenerDetails, callback: (response: { cancel: boolean }) => void): void {
    const refuse = (category: "local_file" | "local_network") => { this.fileRefused(page, category, details.resourceType); callback({ cancel: true }); };
    const verdict = localPageRequest(details.url, details.resourceType, (file) => this.insideCheckout(page, file), page.cdpFileHeld === true);
    if ("cancel" in verdict) {
      if (verdict.cancel) refuse(verdict.boundary === "file" ? "local_file" : "local_network"); else callback({ cancel: false });
      return;
    }
    const known = page.hostVerdicts.get(verdict.resolve);
    if (known !== undefined) { if (known) callback({ cancel: false }); else refuse("local_network"); return; }
    pageSession.resolveHost(verdict.resolve).then((answer) => {
      const publicOnly = answer.endpoints.length > 0 && answer.endpoints.every((endpoint) => !isLocalNetworkHost(endpoint.address.includes(":") ? `[${endpoint.address}]` : endpoint.address));
      if (page.hostVerdicts.size < MAX_HOST_VERDICTS) page.hostVerdicts.set(verdict.resolve, publicOnly);
      if (publicOnly) callback({ cancel: false }); else refuse("local_network");
    }, () => refuse("local_network"));
  }

  /**
   * Whether `file` is in the page's checkout, the one that holds its file
   * (`ResolvedPage.root`), symbolic links resolved, so a link out of it does
   * not carry a read out; for an outside file, in its grant's folder while
   * the grant lasts (D-03, B14).
   */
  private insideCheckout(page: Page, file: string): boolean {
    if (page.grant) return this.externals.inside(page.grant, file);
    if (page.checkoutRoot === undefined) {
      const root = page.route?.root ?? page.workspace.split("\u0000")[1];
      try { page.checkoutRoot = root ? fs.realpathSync.native(root) : null; } catch { page.checkoutRoot = null; }
    }
    if (!page.checkoutRoot) return false;
    let real = file;
    try { real = fs.realpathSync.native(file); } catch { /* a file that is not there answers 404 inside the folder it names */ }
    const relative = path.relative(page.checkoutRoot, real);
    return relative !== ".." && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative);
  }

  /** Whether a checkout's page shows its checkout: any address but a file outside it. */
  private showsCheckout(page: Page, url: string): boolean {
    if (!isFileAddress(url)) return true;
    if (page.cdpShown?.url !== url) page.cdpShown = { url, inside: this.insideGrant(page, url) };
    return page.cdpShown.inside;
  }

  /** Whether a `file:` address is a file of the page's checkout, or of an outside page's grant folder. */
  private insideGrant(page: Page, url: string): boolean {
    try {
      return this.insideCheckout(page, fileURLToPath(url));
    } catch {
      return false;
    }
  }

  private fileRefused(page: Page, category: FileRefusal, resourceType?: Electron.OnBeforeRequestListenerDetails["resourceType"]): void {
    if (page.fileRefusals.has(category)) return;
    page.fileRefusals.add(category);
    this.log.event(FILE_REFUSAL_EVENTS[category], {
      reason: category.startsWith("local_") ? "local_page_boundary" : "cdp_file_boundary", display_id: page.id,
      ...(category.startsWith("local_") ? { boundary: category.slice("local_".length) } : {}),
      ...(resourceType ? { resource_type: resourceType } : {}),
    });
  }

  private load(page: Page, url: string): void {
    if (!loadable(url)) {
      this.log.event("browser.load_refused", { protocol: protocolOf(url) });
      this.update(page, { failure: this.words()("native.browser.addressRefused") });
      return;
    }
    const stamp = page.applied;
    const owned: CoreLoad = { load: stamp, at: this.idle.now(), native: false, committed: false };
    page.loading = owned;
    this.update(page, { url, loading: true, failure: null });
    // An outside file's route is its own file, which no daemon may name; a
    // remote screen's page asks that computer's core.
    const resolved = page.grant ? Promise.resolve<ResolvedPage>({ url, source_url: url, load: stamp })
      : page.source.remote ? this.remoteRoute(page, page.source.remote, url, stamp)
      : this.resolve(page.workspace, page.id, stamp);
    void resolved.then((route) => {
      // The core can canonicalize a checked file address while the loading
      // report is in flight. The load stamp binds this answer to its intent;
      // spelling changes within that generation must not strand the load.
      if (this.pages.get(page.key) !== page || page.applied !== stamp || route.load !== stamp) {
        this.abandoned(page, route);
        return;
      }
      if (!loadable(route.url)) throw new Error("Resolved page address cannot be loaded");
      if (!page.grant && !page.source.remote && browserPartition(page.workspace, route.source_url, this.local.node) !== page.partition) throw new Error("Resolved page requires a different browser partition");
      if (page.source.remote) page.own = (route as ResolvedPage & { own?: boolean }).own === true;
      page.route = route;
      page.checkoutRoot = undefined;
      this.loadStage(page, owned, "browser.load_routed");
      owned.native = true;
      // The call's own promise settles with this load alone, where the page's
      // stop events would also end an earlier load or one the page began.
      return page.view.webContents.loadURL(route.url).then(
        () => {
          this.loadStage(page, owned, "browser.load_stopped", { outcome: "loaded" });
          // An earlier route of a remote screen's page ends with the screen's
          // view: Back never lands on one. Moves inside this route still go
          // back and forth.
          const contents = page.view.webContents;
          if (page.source.remote && routed(route) && !contents.isDestroyed()) contents.navigationHistory.clear();
        },
        (error: unknown) => {
          // did-fail-load reports it; a load a newer one replaced is not a failure.
          const code = (error as { errno?: number }).errno;
          if (code !== ERR_ABORTED) this.log.event("browser.load_failed", { code });
          this.loadStage(page, owned, "browser.load_stopped", code === ERR_ABORTED ? { outcome: "aborted" } : { outcome: "failed", code: code ?? null });
        },
      ).finally(() => {
        if (page.loading === owned) page.loading = null;
      });
    }).catch((error: unknown) => {
      if (page.loading === owned) page.loading = null;
      if (this.pages.get(page.key) !== page || page.applied !== stamp) return;
      const words = error instanceof RemoteRefusal ? error.words : "routeUnavailable";
      this.log.event("browser.route_failed", { detail: error instanceof RemoteRefusal ? error.reason : String(error) });
      this.update(page, { url, loading: false, failure: this.words()(`native.browser.${words}`) });
    });
  }

  /**
   * A route that computer made for a remote screen's page, answered once no
   * page took it (the page closed or was unloaded, or a newer load replaced
   * it while it was asked): it goes now rather than with the screen, unless
   * a page made again for the same display holds it, whose own close lets it
   * go.
   */
  private abandoned(page: Page, route: ResolvedPage): void {
    const remote = page.source.remote;
    if (!remote || !routed(route)) return;
    const current = this.pages.get(page.key);
    if (current && current !== page && current.applied === route.load) return;
    remote.release(page.workspace, page.id, route.load);
  }

  /**
   * Where a remote screen's page loads from: its proxy's state first, then
   * that computer's core's answer for this load (`/page-route`), for the
   * display's own address: a web page loads that address, and a file of
   * that computer the route its daemon made for this screen, on its own
   * loopback, which the proxy carries there (`remoteRouteAddress`, TF1);
   * nothing loads before the answer, and a refusal loads nothing.
   */
  private async remoteRoute(page: Page, remote: RemoteSource, url: string, stamp: number): Promise<ResolvedPage & { own: boolean }> {
    if (url === "about:blank") return { url, source_url: url, load: stamp, own: false };
    const state = await remote.ensure();
    if (state !== "ready") throw new RemoteRefusal(state, egressWords(state));
    const route = await remote.route(page.workspace, page.id, stamp);
    if ("refused" in route) throw new RemoteRefusal(route.refused, routeWords(route.refused));
    // A file page's answer names the same file (its query and fragment are
    // the core's, whose report of a move can land after the question); a
    // web page's, the same address.
    const same = isFileAddress(url) ? sameFileAddress(route.source_url, url) : route.source_url === url;
    if (route.load !== stamp || !same || !remoteRouteAddress(route)) throw new RemoteRefusal("remote_route_invalid", "routeUnavailable");
    return route;
  }

  /**
   * One stage of `owned`, with the time since it was asked, while it is the
   * page's current load. The display id repeats across checkouts, so the
   * page's contents id names the native page, the one a failure dump reads.
   */
  private loadStage(page: Page, owned: CoreLoad, event: "browser.load_routed" | "browser.load_committed" | "browser.load_stopped", fields: Record<string, string | number | null> = {}): void {
    if (page.loading !== owned) return;
    this.log.event(event, { id: page.id, contents: page.contentsId, load: owned.load, ms: this.idle.now() - owned.at, ...fields });
  }

  private watch(page: Page): void {
    const contents = page.view.webContents;
    // `destroy` takes the page out of the map before it closes the contents,
    // so a page still in it lost them some other way.
    contents.once("destroyed", () => {
      if (this.pages.get(page.key) === page) this.closedItself(page);
    });
    const history = () => ({ canGoBack: contents.navigationHistory.canGoBack(), canGoForward: contents.navigationHistory.canGoForward() });
    contents.on("did-start-loading", () => this.update(page, { loading: true, failure: null }));
    contents.on("did-stop-loading", () => {
      page.grab = null;
      this.update(page, { loading: false, ...history() });
    });
    contents.on("did-start-navigation", (details) => {
      if (!details.isMainFrame || details.isSameDocument) return;
      page.grab = { claimed: false };
    });
    // A press in a loading page is the operator choosing it: its keyboard stays.
    contents.on("input-event", (_event, input) => {
      if (page.grab && PRESSES.has(input.type)) page.grab.claimed = true;
      if (PRESSES.has(input.type)) page.pressedAt = this.idle.now();
    });
    contents.on("did-navigate", (_event, url) => {
      // Electron emits this as the navigation finishes, so a commit before
      // the current load's own `loadURL` is the load before it.
      const owned = page.loading;
      if (owned?.native && !owned.committed) {
        this.loadStage(page, owned, "browser.load_committed");
        owned.committed = true;
      }
      // A new document plays nothing yet.
      page.playing = false;
      // A new document takes its host's zoom, and starts without the device size.
      this.update(page, { url: this.sourceAddress(page, url), zoom: contents.getZoomFactor(), ...history() });
      this.emulate(page, true);
      // The limits live in the page's renderer, so each committed document,
      // which may be a new renderer, gets them again.
      void contents.setVisualZoomLevelLimits(...PINCH_ZOOM_LIMITS).catch((error: unknown) => this.log.event("browser.pinch_zoom_failed", { detail: String(error) }));
    });
    contents.on("before-input-event", (event, input) => {
      if (pressKey(input)) page.pressedAt = this.idle.now();
      if (this.routeCycleInput(event, input, page)) return;
      // ⌘+ reaches no menu item (the menu's zoom in is ⌘=), so the page
      // answers it here unless the operator bound that chord to a command.
      if (input.type !== "keyDown" || input.code !== "Equal" || !input.meta || !input.shift || input.alt || input.control) return;
      if (menuHas(Menu.getApplicationMenu()?.items ?? [], ZOOM_IN_ALIAS)) return;
      event.preventDefault();
      this.zoom(page, "in");
    });
    contents.on("did-navigate-in-page", (_event, url, isMainFrame) => {
      if (isMainFrame) this.update(page, { url: this.sourceAddress(page, url), ...history() });
    });
    contents.on("page-title-updated", (_event, title) => this.update(page, { title }));
    contents.on("media-started-playing", () => { page.playing = true; });
    contents.on("media-paused", () => { page.playing = false; });
    contents.on("did-fail-load", (_event, code, description, url, isMainFrame) => {
      if (!isMainFrame || code === ERR_ABORTED) return;
      this.log.event("browser.load_failed", { code, description });
      this.update(page, { url: this.sourceAddress(page, url), loading: false, failure: this.words()("native.browser.loadFailed", { code }) });
    });
    contents.on("render-process-gone", (_event, details) => {
      if (this.cycleInput?.page === page) this.cancelCycle();
      page.playing = false;
      this.log.event("browser.page_gone", { reason: details.reason });
      this.update(page, { loading: false, failure: this.words()("native.browser.stopped") });
    });
    contents.on("blur", () => this.log.event("browser.page_blur", { id: page.id, owed_page: this.returnTo === page, visible: page.visible, window_focused: this.window?.isFocused() ?? false, owed: this.returnTo !== null }));
    contents.on("focus", () => {
      this.log.event("browser.page_focus", { id: page.id, owed_page: this.returnTo === page, visible: page.visible, window_focused: this.window?.isFocused() ?? false, owed: this.returnTo !== null, navigating: page.grab !== null });
      // The page holding the keyboard already is not taking it from anyone.
      if (page.grab && this.keyboard !== page) this.undoGrab(page, page.grab, this.keyboard);
      else this.tookKeyboard(page);
    });
    this.guard(page, contents);
  }

  /** Where the contents of `page`, or of a popup it owns, may go and what may open from them. */
  private guard(page: Page, contents: WebContents): void {
    contents.on("did-frame-navigate", (_event, url) => {
      if (!isFileAddress(url) || page.cdpFileFrame) return;
      page.cdpFileFrame = true;
      this.publishCdp({ contentsId: page.contentsId, reason: "replaced" });
    });
    contents.setWindowOpenHandler(({ url, disposition, features, referrer }) => this.openWindow(page, contents, url, isPopup(disposition, features), referrer.url));
    const forget = () => this.declined.delete(contents.id);
    contents.on("did-navigate", forget);
    contents.once("destroyed", forget);
    const navigation = (event: { preventDefault(): void }, url: string) => {
      // An outside file's page moves within its folder; a file anywhere else
      // is not the operator's to show (B5).
      if (page.grant && isFileAddress(url)) {
        if (!this.insideGrant(page, url)) {
          event.preventDefault();
          this.log.event("browser.navigation_refused", { reason: "external_folder_boundary", grant: page.grant });
        }
        return;
      }
      if (page.fileRestricted && isFileAddress(url)) {
        event.preventDefault();
        this.fileRefused(page, "navigation");
        return;
      }
      // A checkout's page an agent has driven stays in its checkout; the request guard refuses what this event does not see.
      if (page.cdpFileHeld && isFileAddress(url) && !this.insideGrant(page, url)) {
        event.preventDefault();
        this.log.event("browser.navigation_refused", { reason: "checkout_boundary", display_id: page.id });
        return;
      }
      const link = fileLink(page.route, url);
      if (link === "refuse") {
        event.preventDefault();
        this.log.event("browser.navigation_refused", { reason: "remote_file_boundary" });
        return;
      }
      if (link === "beside") {
        event.preventDefault();
        this.emit(page.source, { kind: "open", workspace: page.workspace, id: page.id, url });
        return;
      }
      if (loadable(url)) return;
      event.preventDefault();
      if (appScheme(url)) this.openInApp(page, contents, url);
      else this.log.event("browser.navigation_refused", { protocol: protocolOf(url) });
    };
    contents.on("will-navigate", navigation);
    contents.on("will-frame-navigate", (event) => {
      if (page.fileRestricted && isFileAddress(event.url)) navigation(event, event.url);
    });
    contents.on("will-redirect", navigation);
  }

  /**
   * A page asking for a new window. A sized popup (`window.open` with window
   * features, the way a sign-in button opens one) is a real window that keeps
   * its opener, so the sign-in can post its result back and close itself;
   * a tab (a `target=_blank` link, a shift-click, `window.open` without
   * features) is another browser display, which the core opens.
   */
  private openWindow(page: Page, contents: WebContents, url: string, popup: boolean, referrer: string): WindowOpenHandlerResponse {
    // An outside file's link to another file of its folder opens in its own
    // tab, since no other display may name the file (B5).
    if (page.grant && isFileAddress(url)) {
      if (this.insideGrant(page, url)) void contents.loadURL(url).catch(() => undefined);
      else this.log.event("browser.window_open_refused", { reason: "external_folder_boundary", grant: page.grant });
      return { action: "deny" };
    }
    if (page.fileRestricted && isFileAddress(url)) {
      this.fileRefused(page, "popup");
      return { action: "deny" };
    }
    if (page.cdpFileHeld && isFileAddress(url) && !this.insideGrant(page, url)) {
      this.log.event("browser.window_open_refused", { reason: "checkout_boundary", display_id: page.id });
      return { action: "deny" };
    }
    const link = fileLink(page.route, url);
    if (link === "refuse") {
      this.log.event("browser.window_open_refused", { reason: "remote_file_boundary" });
      return { action: "deny" };
    }
    // A local file page's web address, a sized popup's too, is a display
    // beside it: the file partition holds no web page.
    if (link === "beside") {
      this.emit(page.source, { kind: "open", workspace: page.workspace, id: page.id, url });
      return { action: "deny" };
    }
    if (!loadable(url)) {
      // A frame's window.open names its frame only through the referrer, when its policy leaves one.
      if (appScheme(url)) this.openInApp(page, contents, url, referrer || undefined);
      else this.log.event("browser.window_open_refused", { protocol: protocolOf(url) });
      return { action: "deny" };
    }
    if (popup) return this.popup(page, contents);
    if (url !== "about:blank") this.emit(page.source, { kind: "open", workspace: page.workspace, id: page.id, url: this.sourceAddress(page, url) });
    else this.log.event("browser.window_open_refused", { protocol: "about:" });
    return { action: "deny" };
  }

  /**
   * A popup from a page on screen, at most `MAX_POPUPS` at once. Its window is
   * built here from hide's own options, so of the window's geometry and chrome
   * only a size the page asked for, held to the work area, reaches it: never a
   * frameless, always-on-top, unclosable, modal or off-screen window. It shares its opener's
   * session and, with `outlivesOpener` left false, Electron closes it with its
   * opener, so it ends with its page.
   */
  private popup(page: Page, contents: WebContents): WindowOpenHandlerResponse {
    const parent = this.window;
    if (!parent || parent.isDestroyed()) return { action: "deny" };
    if (!this.shown(page, contents)) {
      this.log.event("browser.popup_refused", { reason: "hidden" });
      return { action: "deny" };
    }
    if (this.popups.size >= MAX_POPUPS) {
      this.log.event("browser.popup_refused", { reason: "cap", live: this.popups.size });
      return { action: "deny" };
    }
    const tile = page.source.remote ? this.popupTile(page, contents, parent) : null;
    if (tile === undefined) return { action: "deny" };
    return {
      action: "allow",
      createWindow: (options) => {
        const bounds = parent.getBounds();
        const asked = { width: options.width, height: options.height };
        const window = new BrowserWindow({
          // A remote screen's popup opens inside its tile, held to it.
          ...(tile ? popupBounds(asked, tile, tile) : popupBounds(asked, bounds, screen.getDisplayMatching(bounds).workArea)),
          parent,
          show: false,
          minimizable: false,
          fullscreenable: false,
          // Electron's preferences for the popup, the opener's with Node, the
          // sandbox and isolation forced, and the contents Chromium made for
          // it: the window must carry them, or the opener is lost.
          webPreferences: options.webPreferences,
          webContents: (options as { webContents?: WebContents }).webContents,
        } as BrowserWindowConstructorOptions);
        this.adopt(page, window, contents.isFocused());
        return window.webContents;
      },
    };
  }

  /**
   * Where a remote screen's page may open a popup (docs/BROWSER_DISPLAYS.md,
   * A page in another computer's screen): only right after the operator
   * pressed in it, while the screen shows and that page holds the keyboard,
   * one a second, at most `REMOTE_BOUNDS.popups` of the screen's, inside the
   * screen's tile on the screen; none in a tile smaller than a popup's
   * least size. `undefined` refuses it, and says why in the log.
   */
  private popupTile(page: Page, contents: WebContents, parent: BrowserWindow): BrowserRect | undefined {
    const source = page.source;
    const area = source.remote?.area();
    const now = this.idle.now();
    const refuse = (reason: string) => {
      this.log.event("browser.popup_refused", { reason });
      return undefined;
    };
    if (!area || page.pressedAt === undefined || now - page.pressedAt > PRESS_WITHIN_MS || !contents.isFocused() || !parent.isFocused() || now - source.popupAt < PRESS_WITHIN_MS) return refuse("no_press");
    if ([...this.popups.values()].filter((popup) => popup.owner.source === source).length >= REMOTE_BOUNDS.popups) return refuse("cap");
    const content = parent.getContentBounds();
    const tile = { x: content.x + area.bounds.x, y: content.y + area.bounds.y, width: area.bounds.width, height: area.bounds.height };
    if (tile.width < POPUP_LEAST.width || tile.height < POPUP_LEAST.height) return refuse("no_room");
    source.popupAt = now;
    return tile;
  }

  /** A popup takes the keyboard only from a page that held it, so a page cannot pull focus from the operator's typing. */
  private adopt(owner: Page, window: BrowserWindow, focus: boolean): void {
    const id = window.webContents.id;
    this.popups.set(id, { window, owner });
    if (owner.source.remote) {
      // Its page moves or sizes nothing: only the operator's own drag takes it out of its tile.
      let logged = false;
      window.webContents.on("content-bounds-updated", (event) => {
        event.preventDefault();
        if (!logged) this.log.event("browser.popup_move_refused", {});
        logged = true;
      });
      window.webContents.setWebRTCIPHandlingPolicy("disable_non_proxied_udp");
      window.webContents.on("input-event", (_event, input) => {
        if (PRESSES.has(input.type)) owner.pressedAt = this.idle.now();
      });
      window.webContents.on("before-input-event", (_event, input) => {
        if (pressKey(input)) owner.pressedAt = this.idle.now();
      });
    }
    window.once("closed", () => {
      this.popups.delete(id);
      this.log.event("browser.popup_closed", { live: this.popups.size });
    });
    this.guard(owner, window.webContents);
    this.present(window, focus);
    this.log.event("browser.popup_opened", { live: this.popups.size });
  }

  /** Whether the contents asking are on screen: a page shown in the front Workspace, or a popup's visible window. */
  private shown(page: Page, contents: WebContents): boolean {
    if (contents.id === page.contentsId) return page.visible;
    return this.popups.get(contents.id)?.window.isVisible() ?? false;
  }

  /** The page a request or a question comes from: a page's own contents, or a popup it owns. */
  private ownerOf(contentsId: number | undefined): Page | undefined {
    if (contentsId === undefined) return undefined;
    for (const page of this.pages.values()) if (page.contentsId === contentsId) return page;
    return this.popups.get(contentsId)?.owner;
  }

  /**
   * A link a page on screen hands to another app on this Mac (`slack:`,
   * `zoommtg:`). It opens there once the operator agrees, the way Chrome asks
   * first, with the asking origin and the link on the question, and Cancel is
   * the default so a stray Return opens nothing; a mail link opens without
   * asking, as it always has. A scheme no app claims, a link from a hidden page
   * or an HTML file preview, one from contents whose last link was declined
   * before they navigated again, and one that arrives while a question is open
   * go no further than the log.
   */
  private openInApp(page: Page, contents: WebContents, url: string, requestingUrl?: string): void {
    const scheme = appScheme(url);
    const refuse = (reason: string) => this.log.event("browser.app_link_refused", { protocol: scheme ?? protocolOf(url), reason });
    if (!scheme) return refuse("not_app_scheme");
    if (page.route && isFileAddress(page.route.source_url)) return refuse("file_page");
    if (!this.shown(page, contents)) return refuse("hidden");
    if (scheme === "mailto:") return void this.handOff(url, scheme);
    const name = app.getApplicationNameForProtocol(url);
    if (!name) return refuse("no_app");
    if (this.declined.has(contents.id)) return refuse("declined");
    if (this.asking) return refuse("asking");
    const sheet = BrowserWindow.fromWebContents(contents) ?? this.window;
    if (!sheet || sheet.isDestroyed()) return refuse("no_window");
    const t = this.words();
    const question = Symbol("app link");
    this.asking = question;
    // A popup that closes under its question never answers it; its window closing does.
    const release = () => { if (this.asking === question) this.asking = null; };
    sheet.once("closed", release);
    const options = {
      type: "question" as const,
      message: t("native.browser.openApp", { name }),
      detail: t("native.browser.appLinkDetail", { origin: requester(requestingUrl ?? this.sourceAddress(page, contents.getURL()), t), name, url: shorten(url) }),
      buttons: [t("native.browser.openAppButton", { name }), t("common.cancel")],
      defaultId: 1,
      cancelId: 1,
      noLink: true,
    };
    dialog.showMessageBox(sheet, options)
      .then(({ response }) => {
        if (response === 0) return this.handOff(url, scheme);
        if (!contents.isDestroyed()) this.declined.add(contents.id);
        this.log.event("browser.app_link_declined", { protocol: scheme });
      })
      .catch((error: unknown) => this.log.event("browser.app_link_failed", { protocol: scheme, detail: String(error) }))
      .finally(() => {
        if (!sheet.isDestroyed()) sheet.removeListener("closed", release);
        release();
      });
  }

  /** The link itself never reaches the log: an app link often carries a sign-in code. */
  private async handOff(url: string, scheme: string): Promise<void> {
    try {
      await shell.openExternal(url);
      this.log.event("browser.app_link_opened", { protocol: scheme });
    } catch {
      this.log.event("browser.app_link_failed", { protocol: scheme });
    }
  }

  /** One held origin and one ordered delivery route, independent of overlay coverage. */
  private routeCycleInput(event: { preventDefault(): void }, input: Input, page?: Page): boolean {
    const held = this.cycleInput;
    if (!held && !page) return false;
    const eventLike = { code: input.code, metaKey: input.meta, altKey: input.alt, shiftKey: input.shift, ctrlKey: input.control };
    const command = input.type === "keyDown" && !input.isComposing ? matchHost(eventLike, this.registry, "electron") : null;
    const starts = !held && command && isCycleCommand(command.id) && page?.visible && page.view.webContents.isFocused();
    const continues = held && command && isCycleCommand(command.id);
    const ends = held && ((input.type === "keyUp" && input.key === held.release) || (input.type === "keyDown" && input.key === "Escape"));
    if (!starts && !continues && !ends) {
      // A cycle chord that reached a page and did not start a hold: say which
      // condition refused it, since the operator sees only a chord that did nothing.
      if (!held && command && isCycleCommand(command.id) && page) {
        this.log.event("browser.cycle_start_refused", { page_visible: page.visible, page_focused: page.view.webContents.isFocused(), window_focused: this.window?.isFocused() ?? false });
      }
      return false;
    }
    const chord = command && hostChord(command, "electron");
    const release = chord && releaseModifier(chord);
    if (starts && release && page) {
      this.cycleSequence = (this.cycleSequence + 1) % Number.MAX_SAFE_INTEGER;
      this.cycleInput = { page, release, cycleId: this.cycleSequence };
      this.returnTo = null;
    }
    const cycle = held ?? this.cycleInput;
    if (!cycle) return false;
    if (ends) this.cycleInput = null;
    event.preventDefault();
    this.emit(cycle.page.source, { kind: "cycle-input", cycleId: cycle.cycleId, workspace: cycle.page.workspace, id: cycle.page.id, type: input.type as "keyDown" | "keyUp", key: input.key, code: input.code, control: input.control, alt: input.alt, meta: input.meta, shift: input.shift });
    // The overlay can be outside the originating page. Transfer its native
    // response at the start, while its logical owner stays frozen in the shell.
    if (starts && this.window?.isFocused()) this.holder(cycle.page.source)?.focus();
    return true;
  }

  private update(page: Page, change: Partial<BrowserPageState>): void {
    page.state = { ...page.state, ...change };
    if (!page.state.loading || page.state.failure) for (const done of [...page.settled]) done();
    if (page.report) return;
    page.report = setTimeout(() => {
      page.report = null;
      if (this.pages.get(page.key) !== page) return;
      this.emit(page.source, { kind: "state", workspace: page.workspace, id: page.id, load: page.applied, state: page.state, ...(page.grant ? { external: true as const } : {}) });
      this.publishCdp();
    }, REPORT_COALESCE_MS);
  }

  private sourceAddress(page: Page, raw: string): string {
    const route = page.route;
    if (!route || !routed(route)) return raw;
    if (raw === route.url) return route.source_url;
    // A routed file page reports a file address alone: an address that does
    // not map back to its route (an ended route, another origin, a path
    // outside the route) keeps the file it last showed, so the display never
    // becomes a web address of a route.
    const file = isFileAddress(route.source_url);
    const kept = isFileAddress(page.state.url) ? page.state.url : route.source_url;
    try {
      const address = new URL(raw);
      const local = new URL(route.url);
      const source = new URL(route.source_url);
      if (address.origin !== local.origin) return file ? kept : raw;
      if (source.protocol === "file:") {
        const workspace = page.workspace.split("\u0000");
        const root = route.root ?? (workspace.length === 2 ? workspace[1] : undefined);
        const prefix = `/${local.pathname.split("/")[1]}/`;
        if (!root || !address.pathname.startsWith(prefix)) return kept;
        return wireFileAddress(root, decodeURIComponent(address.pathname.slice(prefix.length)), address.search, address.hash) ?? kept;
      }
      address.host = source.host;
      address.protocol = source.protocol;
      return address.href;
    } catch { return raw; }
  }

  /** Tells the shell that placed a source's pages: this window's, or a remote screen's page. */
  private emit(source: Source, event: BrowserHostEvent): void {
    const contents = this.holder(source);
    if (contents && !contents.isDestroyed()) contents.send(source.remote ? REMOTE_BROWSER_EVENT_CHANNEL : BROWSER_EVENT_CHANNEL, event);
  }

  /** The contents of the shell that placed a source's pages, which hears them and takes the keyboard back from them. */
  private holder(source: Source): WebContents | null {
    const contents = source.remote ? source.remote.contents() : (this.window?.webContents ?? null);
    return contents && !contents.isDestroyed() ? contents : null;
  }

  private show(page: Page, visible: boolean): void {
    if (page.visible === visible) return;
    // A hidden WebContentsView is no longer a native keyboard responder.
    // Keep the held cycle's remaining OS input in this same window: the
    // shell preserves its logical page owner and consumes release/Escape.
    // A page that had the keyboard hands it to the shell, whose layer over
    // the page (a note written on it, a dialog) is why it hides.
    const keyboard = !visible && page.view.webContents.isFocused();
    page.visible = visible;
    page.view.setVisible(visible);
    this.log.event("browser.page_visible", { id: page.id, visible, owed_page: this.returnTo === page, held: this.cycleInput?.page === page });
    if (visible && this.returnTo === page) this.giveBack();
    if (!visible && (keyboard || this.cycleInput?.page === page) && this.window?.isFocused()) this.holder(page.source)?.focus();
  }

  /** The host gives `page` the keyboard on purpose, so a navigation in flight keeps it there. */
  private focusPage(page: Page): void {
    if (page.grab) page.grab.claimed = true;
    page.view.webContents.focus();
  }

  private tookKeyboard(page: Page): void {
    this.keyboard = page;
    // Another page holding the keyboard settles the debt: it is not the page the hold began on.
    if (this.returnTo && this.returnTo !== page) this.returnTo = null;
    if (page.visible) this.emit(page.source, { kind: "focus", workspace: page.workspace, id: page.id });
  }

  /**
   * A navigation's commit focused `page` while `from` (a page, or the shell
   * as null) held the keyboard. The press that would claim it
   * reaches `input-event` after the native focus it causes, in the same
   * turn, so the keyboard goes back on the next one unless a press came.
   * A window in the background is left alone: macOS gives it back the
   * focus it stored on blur when it returns, and focusing contents there
   * would bring the window forward.
   */
  private undoGrab(page: Page, grab: NavigationGrab, from: Page | WebContents | null): void {
    setImmediate(() => {
      if (this.pages.get(page.key) !== page || page.view.webContents.isDestroyed() || !page.view.webContents.isFocused()) return;
      if (grab.claimed) return this.tookKeyboard(page);
      const window = this.window;
      if (!window?.isFocused() || window.webContents.isDestroyed()) return;
      // The keyboard goes back to whoever held it: a page, a remote screen's view, or this window's shell.
      const held = from && "view" in from ? (this.pages.get(from.key) === from && from.visible ? from.view.webContents : null) : from && !from.isDestroyed() ? from : null;
      (held ?? window.webContents).focus();
      const to = from && "view" in from ? (held ? from.id : "shell") : held ? "remote_screen" : "shell";
      this.log.event("browser.navigation_focus_returned", { id: page.id, to });
    });
  }

  /** Closes a source's hidden pages past its bound, least recently shown first, whatever holds them (B2). */
  /**
   * A remote source at its bound with a display it places needing a page:
   * the source's page shown longest ago among those this sync does not place
   * (`placed`, the sync's own set, never `page.placed`, which a display placed
   * later in the same sync has not set yet) closes, as a page past its bound
   * does, so only displays all on screen at once are refused. False when
   * every page is placed.
   */
  private evictHidden(source: Source, placed: ReadonlySet<string>): boolean {
    let oldest: Page | null = null;
    for (const page of this.pages.values()) {
      if (page.source !== source || placed.has(page.key)) continue;
      if (!oldest || page.shownAt < oldest.shownAt) oldest = page;
    }
    if (!oldest) return false;
    this.destroy(oldest, "evicted");
    return true;
  }

  private evictOverCap(source: Source): void {
    const cap = source.remote ? REMOTE_BOUNDS.pages : MAX_LIVE_VIEWS;
    for (const key of overCap([...this.pages.values()].filter((page) => page.source === source), cap)) {
      const page = this.pages.get(key);
      if (page) this.destroy(page, "evicted");
    }
  }

  /**
   * Whether closing the page now would break something the operator or an
   * agent sees: it is on screen (drawn in the front Workspace, even with the
   * window minimized or covered), plays media, downloads, is driven over CDP
   * (Comment included) or has a popup open (B3, B5).
   */
  private held(page: Page): boolean {
    if (page.visible || page.placed || page.playing || page.downloads > 0) return true;
    // A second player still heard after the first one paused.
    if (page.view.webContents.isCurrentlyAudible()) return true;
    if (this.cdpLeases.has(page.contentsId)) return true;
    for (const popup of this.popups.values()) if (popup.owner === page) return true;
    return false;
  }

  /** The idle check: a held page starts its bound again, and one hidden and idle past it is unloaded (B1, B4). */
  private unloadIdle(): void {
    const now = this.idle.now();
    const views = [...this.pages.values()].map((page) => {
      const held = this.held(page);
      if (held) page.idleSince = now;
      return { key: page.key, held, idleSince: page.idleSince };
    });
    const expired = idleExpired(views, now, this.idle.bound);
    for (const key of expired) {
      const page = this.pages.get(key);
      if (page) this.destroy(page, "idle");
    }
  }

  /**
   * An agent's connection to a display: an unloaded web page loads its
   * address again, hidden, with the screen, its selection and its areas left
   * as they are, and the connection waits for that load (B10). One already
   * loading again is waited on the same way, so a second connection drives
   * no half-loaded page. A load that failed stays failed on its tab, and the
   * agent's next connection loads it again (B11). `none` is a display with
   * nothing to load again here: a live page, a pending one, a file.
   */
  /**
   * A picture of a display's page for a phone (hided `mobile/page_shot.rs`):
   * Electron's capture of what the page draws, in front or behind another
   * tab, at most `STILL_PIXELS` and as a JPEG. A read: it shows, selects,
   * focuses and loads nothing, so a page the host unloaded has no picture
   * (`not_loaded`), and one laid out off screen and never shown (B32) gives
   * an empty image (`not_drawn`). Electron counts the page visible while it
   * captures (`stayHidden` false), which may be why it draws a page behind a
   * tab where the debugger's capture does not; a capture past `STILL_WAIT_MS`
   * answers `capture_timeout`. Its
   * area must be one the core authorized, as a connection's is.
   */
  async still(scope: CdpScope): Promise<{ jpeg: Uint8Array } | { failure: string }> {
    const key = scope.display_id ? viewKey(scope.workspace, scope.display_id) : null;
    if (!key || this.cdpAreas.get(key) !== scope.area_id) return { failure: "display_missing" };
    const page = this.pages.get(key);
    if (!page || page.view.webContents.isDestroyed()) return { failure: "not_loaded" };
    let timer: NodeJS.Timeout | undefined;
    const image = await Promise.race([
      page.view.webContents.capturePage(),
      new Promise<null>((resolve) => { timer = setTimeout(() => resolve(null), STILL_WAIT_MS); }),
    ]).finally(() => clearTimeout(timer));
    if (image === null) return { failure: "capture_timeout" };
    if (image.isEmpty()) return { failure: "not_drawn" };
    const { width, height } = image.getSize();
    const scale = Math.min(1, Math.sqrt(STILL_PIXELS / (width * height)));
    const fitted = scale < 1 ? image.resize({ width: Math.max(1, Math.round(width * scale)), quality: "good" }) : image;
    return { jpeg: new Uint8Array(fitted.toJPEG(STILL_QUALITY)) };
  }

  /**
   * Loads a display's page again, hidden, for an agent's connection to it: one this host
   * unloaded, by its record, or an outside page by `grant`, which hided read from the core's
   * display, since no inventory row names it; this window holds the grant while a tab stands on it.
   * A file page, of a checkout or outside, loads again only for a file capability (`file`).
   */
  async cdpRevive(scope: CdpScope, file: boolean, grant?: string): Promise<"none" | "loaded" | "failed"> {
    const window = this.window;
    if (!window || !scope.display_id) return "none";
    const key = viewKey(scope.workspace, scope.display_id);
    const record: Unloaded | undefined = grant === undefined ? this.unloaded.get(key)
      : file && this.externals.pageUrl(grant) ? { workspace: scope.workspace, id: scope.display_id, area_id: scope.area_id, url: "", load: 0, device: null, bounds: null } : undefined;
    if (!record || record.area_id !== scope.area_id || this.cdpAreas.get(key) !== scope.area_id) return "none";
    if (!file && isFileAddress(record.url)) return "none";
    let page = this.pages.get(key);
    if (page && !page.state.loading && !page.state.failure) {
      this.unloaded.delete(key);
      return "none";
    }
    if (!page || page.state.failure) {
      const display = { id: record.id, area_id: record.area_id, url: record.url, load: record.load, rect: null, visible: false, ...(grant ? { grant } : {}) };
      if (page) this.load(page, this.addressOf(display));
      else {
        page = this.create(this.local, window, record.workspace, display);
        page.device = record.device;
        // A page never shown here lays out where an off-screen load would (B32).
        page.bounds = record.bounds ?? offScreenBounds(window.getContentBounds());
        page.offScreen = true;
        page.view.setBounds(page.bounds);
        this.emulate(page);
      }
      // Loaded again counts as seen, so the next check does not close it at once.
      page.shownAt = page.idleSince = this.idle.now();
      this.log.event("browser.reloaded_for_agent", { id: page.id });
      this.evictOverCap(this.local);
      this.publishCdp();
    }
    await this.waitSettled(page);
    if (this.pages.get(key) !== page || page.state.failure) return "failed";
    // A page still loading past the wait is driven as it is; its record stays until it settles, so a failure later is loaded again too.
    if (!page.state.loading) this.unloaded.delete(key);
    return "loaded";
  }

  /** Resolves once the page's load settles, it closes, or `REVIVE_WAIT_MS` passes. */
  private waitSettled(page: Page): Promise<void> {
    if (!page.state.loading || page.state.failure) return Promise.resolve();
    return new Promise((done) => {
      const finish = () => {
        clearTimeout(timer);
        page.settled.delete(finish);
        done();
      };
      const timer = setTimeout(finish, REVIVE_WAIT_MS);
      page.settled.add(finish);
    });
  }

  private hideAll(source: Source): void {
    for (const page of this.pages.values()) if (page.source === source) this.show(page, false);
  }

  private destroy(page: Page, reason: Closing): void {
    const unloading = reason === "evicted" || reason === "idle";
    if (unloading) {
      // The core reports the page `unloaded` until it shows or an agent drives it again.
      this.emit(page.source, { kind: "gone", workspace: page.workspace, id: page.id, load: page.applied, url: page.state.url, ...(page.grant ? { external: true as const } : {}) });
      const area_id = this.cdpAreas.get(page.key);
      // A web page and a checkout's file page enter CDP, so either can be loaded again for an agent.
      if (area_id && !page.grant && (!page.fileRestricted || !isFileAddress(page.state.url))) {
        this.unloaded.set(page.key, { workspace: page.workspace, id: page.id, area_id, url: page.state.url, load: page.applied, device: page.device, bounds: page.bounds });
      }
      this.log.event("browser.unloaded", { id: page.id, reason: reason === "idle" ? "idle" : "cap" });
    }
    for (const done of [...page.settled]) done();
    if (this.cycleInput?.page === page) this.cancelCycle();
    if (this.returnTo === page) this.returnTo = null;
    if (this.keyboard === page) this.keyboard = null;
    this.pages.delete(page.key);
    // A remote screen's page was never a CDP target.
    if (!page.source.remote) this.publishCdp({ contentsId: page.contentsId, reason: reason === "idle" ? "evicted" : reason });
    // An outside file's page resolved no route, so the daemon has none to let
    // go; a remote screen's page that loaded from a route lets that computer's go.
    if (!page.grant && !page.source.remote) this.release(page.workspace, page.id, page.applied);
    if (page.source.remote && page.route && routed(page.route)) page.source.remote.release(page.workspace, page.id, page.route.load);
    if (page.report) clearTimeout(page.report);
    const window = this.window;
    if (window && !window.isDestroyed()) window.contentView.removeChildView(page.view);
    // Closing the contents ends the page's renderer process; a view left
    // alone keeps it running after the view is gone.
    const contents = page.view.webContents as WebContents | undefined;
    if (contents && !contents.isDestroyed()) contents.close();
    this.log.event("browser.view_closed", { reason, live: this.pages.size });
  }
}

/** A popup's least size, as `popupBounds` holds one: a tile smaller than it has no room for one. */
const POPUP_LEAST = { width: 320, height: 240 };

/** Whether a key is the operator's press, as a click is: Enter or Space going down. */
function pressKey(input: Input): boolean {
  return input.type === "keyDown" && !input.isAutoRepeat && (input.key === "Enter" || input.key === " ");
}

function zoomContents(contents: WebContents, zoom: PageZoom): void {
  contents.setZoomFactor(nextZoomFactor(contents.getZoomFactor(), zoom));
}

/** A link as the question shows it, cut to a length a sheet holds. */
function shorten(url: string): string {
  return url.length > 160 ? `${url.slice(0, 159)}…` : url;
}

/** Who is asking, as the origin the operator would recognize, or a page with none. */
function requester(url: string, t: TFunction<"translation">): string {
  try {
    const origin = new URL(url).origin;
    if (origin !== "null") return origin;
  } catch { /* falls through */ }
  return t("native.browser.thisPage");
}

function menuHas(items: readonly MenuItem[], accelerator: string): boolean {
  return items.some((item) => item.accelerator === accelerator || (item.submenu ? menuHas(item.submenu.items, accelerator) : false));
}

function isFileAddress(url: string): boolean {
  return protocolOf(url) === "file:";
}

/** The session of a page whose grant is gone, which loads nothing and shares nothing (a grant's own come from `ExternalFiles.partition`). */
const GONE_GRANT_PARTITION = "hide-browser-external-gone";

/** Why a remote screen's page does not load, with the words (`native.browser.*`) its slot shows. */
class RemoteRefusal extends Error {
  constructor(readonly reason: string, readonly words: string) {
    super(reason);
  }
}

/** The words of a refused route answer. */
function routeWords(reason: string): string {
  switch (reason) {
    case "remote_device_page_unavailable":
      return "remoteDevicePage";
    // Only an older Hide there refuses its own files.
    case "remote_file_unavailable":
    case "remote_update_there":
      return "remoteUpdateThere";
    case "browser_route_limit":
      return "remotePageLimit";
    case "view_unavailable":
    case "route_unavailable":
    case "route_busy":
      return "routeUnavailable";
    default:
      return egressWords(reason);
  }
}
