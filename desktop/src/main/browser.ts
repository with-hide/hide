// Browser displays (issue 155): one WebContentsView per browser display the
// shell has shown, placed over the rect the shell reports through the
// hideHost bridge. The core owns which displays exist and the URL each was
// asked to load; this owns the pages. A page lives in a browser session
// partition with no preload and no Node, so nothing in it can reach the
// shell's bridge or the daemon's token.
//
//   shown      -> created on first show, then placed and made visible
//   not shown  -> hidden, still alive; a move between areas never reloads
//   closed     -> the core's inventory no longer names it: destroyed,
//                 which ends its renderer process
//
// A hidden page past `MAX_LIVE_VIEWS` is closed and loads again when shown.
//
// A sized popup a page on screen opens (a sign-in window) is a real child
// window that keeps its opener, belongs to that page and closes with it; a
// link to another app leaves only after the operator agrees.

import { app, BrowserWindow, dialog, ipcMain, Menu, screen, session, shell, WebContentsView, type BrowserWindowConstructorOptions, type Input, type IpcMainEvent, type IpcMainInvokeEvent, type MenuItem, type Session, type WebContents, type WindowOpenHandlerResponse } from "electron";
import fs from "node:fs";
import path from "node:path";
import type { TFunction } from "i18next";
import { fileURLToPath, pathToFileURL } from "node:url";
import type { BrowserDevicePlacement, BrowserHostEvent, BrowserPageState, BrowserPlacement, BrowserRect } from "../../../web/src/host";
import { hostChord, isCycleCommand, keySystemOf, matchHost, releaseModifier, systemRegistry, type Command, type CommandId } from "../../../web/src/shortcuts";
import { BROWSER_CAPTURE_CHANNEL, BROWSER_COMMAND_CHANNEL, BROWSER_CYCLE_END_CHANNEL, BROWSER_EVENT_CHANNEL, BROWSER_SYNC_CHANNEL } from "../channel";
import { appScheme, browserPartition, deviceEmulation, isLocalNetworkHost, isPopup, loadable, localPageRequest, MAX_AUTHORIZED_SCOPES, MAX_LIVE_VIEWS, MAX_POPUPS, overCap, parseCommand, parseSync, parseTarget, popupBounds, remoteRequest, toBounds, viewKey } from "./browserSync";
import { nextZoomFactor, type PageZoom } from "../../../web/src/zoomSteps";
import type { HostLog } from "./log";
import type { CdpPage, CdpRetirement, CdpScope } from "./browserCdp";
import { accelerator } from "./menu";
import type { ExternalFiles } from "./externalFiles";

/** Page state reports coalesce to one per view in this window, so a title ticking every frame costs one event. */
const REPORT_COALESCE_MS = 100;
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

type Page = {
  key: string;
  workspace: string;
  id: string;
  view: WebContentsView;
  /** The load stamp this page last loaded. */
  applied: number;
  visible: boolean;
  /** Placed on its slot in the front Workspace, shown or standing behind its still for an overlay. */
  placed: boolean;
  shownAt: number;
  state: BrowserPageState;
  /** The device size the page lays out at, null to fit its slot, with the bounds it was last placed at. */
  device: BrowserDevicePlacement | null;
  bounds: BrowserRect | null;
  /** The emulation last applied to its contents, as JSON; null when none is. */
  emulated: string | null;
  report: NodeJS.Timeout | null;
  route: ResolvedPage | null;
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
  /** Three finite guard categories bound diagnostic state and output even
   * when a renderer repeatedly attempts file requests or opens windows. */
  readonly fileRefusals: Set<FileRefusal>;
  /** A local file page's checkout, resolved once: what its files may load from. */
  checkoutRoot?: string | null;
  /**
   * An outside HTML file's grant (PRD outside-path-in-hide): the page loads
   * the granted file in a session of its own and its files from the grant's
   * folder alone, and it never enters CDP or reports its address to the core.
   */
  readonly grant?: string;
  /** What each public name a local file page asked for resolved to, bounded. */
  readonly hostVerdicts: Map<string, boolean>;
};
/** Names a local file page may have resolved at once; past it a name is resolved on every request. */
const MAX_HOST_VERDICTS = 64;

export type ResolvedPage = { url: string; source_url: string; load: number };

/** A popup window a page opened, under the page that owns it (for a popup's own popup, the same page). */
type Popup = { window: BrowserWindow; owner: Page };

export class BrowserViews {
  private readonly pages = new Map<string, Page>();
  /** Area membership comes from the core's retained inventory, including hidden workspaces. */
  private cdpAreas = new Map<string, string>();
  private cdpChanged: ((retirement?: CdpRetirement) => void) | null = null;
  private cdpObserved: { key: string; area: string; contents: number; url: string; title: string }[] = [];
  private readonly cdpLeases = new Map<number, CdpPage>();
  private attachmentEpoch: string | null = null;
  /** The core's own node id as the shell last named it; its pages load this computer's loopback directly. */
  private node: string | null = null;
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
    this.emit({ kind: "window-focus", focused });
  }
  /**
   * Whether the window has had its `focus` event since its last `blur`. On
   * macOS that event is the window becoming main again, and Electron restores
   * the focus it stored on `blur` just before it; a window already key but not
   * yet back would hand that restore the keyboard a page was just given.
   */
  private windowBack = true;

  setRegistry(registry: readonly Command[]): void { this.registry = registry; }

  /** Pays the keyboard owed to the page a blur cancelled a hold on, once the window is back and the page shown. */
  private giveBack(): void {
    const page = this.returnTo;
    if (!page) return;
    if (!page.visible || !this.windowBack) return;
    this.returnTo = null;
    page.view.webContents.focus();
    this.log.event("browser.window_return", { id: page.id, page_focused: page.view.webContents.isFocused() });
  }

  cdpIncarnation(scope: CdpScope): number | null { return this.cdpScopes.get(JSON.stringify([scope.workspace, scope.area_id])) ?? null; }

  /**
   * Only display pages enter CDP. The shell and popup windows are absent. A
   * page of the file partition is listed as `local`, which the gateway shows
   * to the comment capability alone (BROWSER_DISPLAYS.md, Comment mode).
   */
  cdpPages(): CdpPage[] {
    const result: CdpPage[] = [];
    for (const page of this.pages.values()) {
      const area_id = this.cdpAreas.get(page.key);
      const contents = page.view.webContents;
      // An outside file is the operator's alone: no capability, Comment's
      // included, ever reaches it (B18).
      if (!area_id || contents.isDestroyed() || page.grant) continue;
      if (!page.fileRestricted) result.push({ workspace: page.workspace, area_id, id: page.id, local: true, contents });
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
      && [...this.pages.values()].some((page) => page.key === previous.key && page.view.webContents.id === previous.contents));
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
  cdpAttached(page: CdpPage, attached: boolean): void {
    const owner = this.ownerOf(page.contents.id);
    if (attached && owner) owner.cdpRestricted = true;
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
    if (held) this.emit({ kind: "cycle-cancel", cycleId: held.cycleId, workspace: held.page.workspace, id: held.page.id });
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
  ) {
    ipcMain.on(BROWSER_CYCLE_END_CHANNEL, (event, cycleId: unknown) => {
      if (!this.trusted(event)) return this.log.event("browser.ipc_refused", { channel: "cycle_end" });
      if (this.cycleInput?.cycleId === cycleId) this.cycleInput = null;
    });
    ipcMain.on(BROWSER_SYNC_CHANNEL, (event, value: unknown) => {
      if (!this.trusted(event)) return this.log.event("browser.ipc_refused", { channel: "sync" });
      const sync = parseSync(value);
      if (!sync) return this.log.event("browser.sync_invalid", {});
      if (sync.node) this.node = sync.node;
      // UI owns this optional wire field. Its parser forwards it after the
      // shared contract lands; older shells have no replay epoch.
      const authority = sync as typeof sync & { attachment_epoch?: string; authorized_scopes?: { workspace: string; area_id: string; incarnation?: number }[] };
      this.sync(sync.workspace, sync.displays, sync.retained, authority.attachment_epoch, authority.authorized_scopes);
      if (sync.external_grants) this.keepGrants(new Set(sync.external_grants));
    });
    ipcMain.handle(BROWSER_CAPTURE_CHANNEL, async (event, value: unknown) => {
      if (!this.trusted(event)) return null;
      const target = parseTarget(value);
      const page = target ? this.pages.get(viewKey(target.workspace, target.id)) : undefined;
      // A covered page's still: the shell asks while the page still shows,
      // ahead of the sync that hides it, and again for a page it keeps covered
      // in the front Workspace whose zoom moved under the zoom popover.
      if (!page || (!page.visible && !page.placed)) {
        this.log.event("browser.capture_refused", { id: target?.id ?? null, reason: page ? "hidden" : "missing" });
        return null;
      }
      try {
        const image = await page.view.webContents.capturePage();
        if (!image.isEmpty()) return image.toDataURL();
        this.log.event("browser.capture_empty", { id: page.id });
        return null;
      } catch (error) {
        this.log.event("browser.capture_failed", { id: page.id, detail: String(error) });
        return null;
      }
    });
    ipcMain.on(BROWSER_COMMAND_CHANNEL, (event, value: unknown, name: unknown) => {
      if (!this.trusted(event)) return this.log.event("browser.ipc_refused", { channel: "command" });
      const target = parseTarget(value);
      const command = parseCommand(name);
      const page = target ? this.pages.get(viewKey(target.workspace, target.id)) : undefined;
      if (!page || !command) return;
      const contents = page.view.webContents;
      const zoom = SHELL_ZOOM_COMMANDS[command];
      if (zoom) this.zoom(page, zoom);
      else if (command === "focus") {
        if (page.visible && this.window?.isFocused()) contents.focus();
      } else if (command === "back" && contents.navigationHistory.canGoBack()) contents.navigationHistory.goBack();
      else if (command === "forward" && contents.navigationHistory.canGoForward()) contents.navigationHistory.goForward();
      else if (command === "stop") contents.stop();
      else if (command === "reload") {
        // A page that failed to load has nothing to reload; its address loads again.
        if (page.state.failure) this.load(page, page.state.url);
        else contents.reload();
      }
    });
  }

  /** The window the pages are drawn in. A new window starts with no pages. */
  attach(window: BrowserWindow): void {
    this.window = window;
    // Once a native page starts a hold, its shell input uses the same IPC
    // route. Even a quick release cannot overtake the forwarded first chord.
    window.webContents.on("before-input-event", (event, input) => this.routeCycleInput(event, input));
    // A shell that loads again, or moves to another daemon, has not placed
    // anything yet: its pages hide until it says where they go.
    window.webContents.on("did-start-navigation", (details) => {
      if (details.isMainFrame && !details.isSameDocument) this.hideAll();
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
    window.webContents.on("focus", () => this.log.event("browser.shell_focus", { window_focused: window.isFocused(), owed: this.returnTo !== null }));
    // macOS reports the window hidden while nothing of it is drawn (covered,
    // or ordered out), and the shell then draws no frame, so it sends no sync.
    window.on("show", () => this.log.event("browser.window_visible", { visible: true, owed: this.returnTo !== null }));
    window.on("hide", () => this.log.event("browser.window_visible", { visible: false, owed: this.returnTo !== null }));
    window.on("closed", () => {
      this.cancelCycle();
      for (const page of [...this.pages.values()]) this.destroy(page, "window_closed");
      if (this.window === window) this.window = null;
    });
  }

  /** Whether a page holds the keyboard, so an app-menu command gives it back to the shell first. */
  hasFocus(): boolean {
    return this.focused() !== undefined;
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
    const params = page.device && page.bounds ? deviceEmulation(page.device, page.bounds, contents.getZoomFactor()) : null;
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

  private sync(workspace: string | null, displays: BrowserPlacement[], retained: { workspace: string; id: string; area_id?: string }[], attachmentEpoch?: string, authorizedScopes?: { workspace: string; area_id: string; incarnation?: number }[]): void {
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
    const placed = new Set(displays.map((display) => viewKey(workspace ?? "", display.id)));
    for (const page of [...this.pages.values()]) {
      if (!listed.has(page.key)) this.destroy(page, "closed");
      else if (page.workspace !== workspace || !placed.has(page.key)) {
        // A retained page outside the front presentation stays alive and hidden.
        if (this.returnTo === page) this.returnTo = null;
        page.placed = false;
        this.show(page, false);
      }
    }
    const now = Date.now();
    const zoom = window.webContents.getZoomFactor();
    for (const display of displays) {
      const key = viewKey(workspace ?? "", display.id);
      let page = this.pages.get(key);
      if (!display.rect) {
        if (page) {
          page.placed = false;
          this.show(page, false);
        }
        continue;
      }
      if (!page) page = this.create(window, workspace ?? "", display);
      else if (page.partition !== this.partitionOf(workspace ?? "", display)) {
        this.destroy(page, "replaced");
        page = this.create(window, workspace ?? "", display);
      } else if (display.load > page.applied) {
        page.applied = display.load;
        this.load(page, this.addressOf(display));
      }
      const bounds = toBounds(display.rect, zoom);
      page.view.setBounds(bounds);
      page.placed = true;
      page.device = display.device ?? null;
      page.bounds = bounds;
      this.emulate(page);
      this.show(page, display.visible);
      page.shownAt = now;
    }
    for (const key of overCap([...this.pages.values()], MAX_LIVE_VIEWS)) {
      const page = this.pages.get(key);
      if (page) this.destroy(page, "evicted");
    }
    this.publishCdp();
    if (attachmentEpoch && /^[A-Za-z0-9-]{1,64}$/.test(attachmentEpoch) && attachmentEpoch !== this.attachmentEpoch) {
      this.attachmentEpoch = attachmentEpoch;
      for (const page of this.pages.values()) this.sendCdpAttached(page, this.cdpLeases.has(page.view.webContents.id));
    }
  }

  /**
   * An outside file's own session, kept in memory and shared with no other
   * page (B4); every other page's from its address (`browserPartition`).
   */
  private partitionOf(workspace: string, display: BrowserPlacement): string {
    return display.grant ? externalPartition(display.grant) : browserPartition(workspace, display.url, this.node);
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
    const gone = new Set(this.externals.keep(named));
    if (gone.size === 0) return;
    for (const page of [...this.pages.values()]) if (page.grant && gone.has(page.grant)) this.destroy(page, "closed");
    for (const grant of gone) {
      const partition = externalPartition(grant);
      if (!this.configuredSessions.has(partition)) continue;
      this.configuredSessions.delete(partition);
      void session.fromPartition(partition).clearStorageData().catch((error: unknown) => this.log.event("external.storage_clear_failed", { grant, detail: String(error) }));
    }
  }

  private create(window: BrowserWindow, workspace: string, display: BrowserPlacement): Page {
    const partition = this.partitionOf(workspace, display);
    const view = new WebContentsView({
      webPreferences: { session: this.sessionFor(partition), sandbox: true, contextIsolation: true, nodeIntegration: false, webSecurity: true },
    });
    const page: Page = {
      key: viewKey(workspace, display.id),
      workspace,
      id: display.id,
      view,
      applied: display.load,
      visible: false,
      placed: false,
      shownAt: 0,
      state: { url: display.url, title: "", loading: true, canGoBack: false, canGoForward: false, failure: null, zoom: 1 },
      device: null,
      bounds: null,
      emulated: null,
      report: null,
      route: null,
      partition,
      fileRestricted: !partition.startsWith("persist:hide-browser-file-") && !display.grant,
      cdpRestricted: false,
      cdpDownloadRefused: false,
      cdpFileFrame: false,
      fileRefusals: new Set(),
      hostVerdicts: new Map(),
      ...(display.grant ? { grant: display.grant } : {}),
    };
    view.setVisible(false);
    window.contentView.addChildView(view);
    this.pages.set(page.key, page);
    this.watch(page);
    this.load(page, this.addressOf(display));
    this.log.event("browser.view_created", { live: this.pages.size });
    return page;
  }

  private sessionFor(partition: string): Session {
    const pageSession = session.fromPartition(partition);
    if (this.configuredSessions.has(partition)) return pageSession;
    this.configuredSessions.add(partition);
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
      if (page?.cdpRestricted) {
        event.preventDefault();
        if (!page.cdpDownloadRefused) {
          page.cdpDownloadRefused = true;
          this.log.event("browser.download_refused", { reason: "cdp_filesystem_boundary", display_id: page.id });
        }
      } else this.log.event("browser.download", { mime: item.getMimeType() });
    });
    pageSession.webRequest.onBeforeRequest({ urls: ["<all_urls>"] }, (details, callback) => {
      const page = this.ownerOf(details.webContentsId);
      if (page?.fileRestricted && isFileAddress(details.url)) {
        this.fileRefused(page, "request", details.resourceType);
        callback({ cancel: true });
        return;
      }
      if (page?.route && isFileAddress(page.route.url) && isFileAddress(page.route.source_url)) {
        this.localPageRequest(pageSession, page, details, callback);
        return;
      }
      const outcome = !page
        ? partition === "persist:hide-browser-web" ? remoteRequest({ source_url: details.url, url: details.url }, details.url) : { cancel: true }
        : !page.route ? { cancel: true }
        : this.node !== null && page.workspace.startsWith(`${this.node}\u0000`) ? {}
        : remoteRequest(page.route, details.url);
      callback(outcome);
    });
    return pageSession;
  }

  /**
   * A local HTML page's request, held to what a browser allows a local page
   * (`localPageRequest`). A public name is resolved before it is allowed, and
   * one that answers with a private address is refused; the answer is kept
   * for the generation, so a name that changes its answer later is not seen.
   */
  private localPageRequest(pageSession: Session, page: Page, details: Electron.OnBeforeRequestListenerDetails, callback: (response: { cancel: boolean }) => void): void {
    const refuse = (category: "local_file" | "local_network") => { this.fileRefused(page, category, details.resourceType); callback({ cancel: true }); };
    const verdict = localPageRequest(details.url, details.resourceType, (file) => this.insideCheckout(page, file));
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
   * Whether `file` is in the page's checkout, symbolic links resolved, so a
   * link out of it does not carry a read out; for an outside file, in its
   * grant's folder while the grant lasts (D-03, B14).
   */
  private insideCheckout(page: Page, file: string): boolean {
    if (page.grant) return this.externals.inside(page.grant, file);
    if (page.checkoutRoot === undefined) {
      const root = page.workspace.split("\u0000")[1];
      try { page.checkoutRoot = root ? fs.realpathSync.native(root) : null; } catch { page.checkoutRoot = null; }
    }
    if (!page.checkoutRoot) return false;
    let real = file;
    try { real = fs.realpathSync.native(file); } catch { /* a file that is not there answers 404 inside the folder it names */ }
    const relative = path.relative(page.checkoutRoot, real);
    return relative !== ".." && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative);
  }

  /** Whether a `file:` address is a file of an outside page's grant folder. */
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
    this.update(page, { url, loading: true, failure: null });
    // An outside file's route is its own file, which no daemon may name.
    const resolved = page.grant ? Promise.resolve<ResolvedPage>({ url, source_url: url, load: stamp }) : this.resolve(page.workspace, page.id, stamp);
    void resolved.then((route) => {
      // The core can canonicalize a checked file address while the loading
      // report is in flight. The load stamp binds this answer to its intent;
      // spelling changes within that generation must not strand the load.
      if (this.pages.get(page.key) !== page || page.applied !== stamp || route.load !== stamp) return;
      if (!loadable(route.url)) throw new Error("Resolved page address cannot be loaded");
      if (!page.grant && browserPartition(page.workspace, route.source_url, this.node) !== page.partition) throw new Error("Resolved page requires a different browser partition");
      page.route = route;
      return page.view.webContents.loadURL(route.url).catch((error: unknown) => {
        // did-fail-load reports it; a load a newer one replaced is not a failure.
        const code = (error as { errno?: number }).errno;
        if (code !== ERR_ABORTED) this.log.event("browser.load_failed", { code });
      });
    }).catch((error: unknown) => {
      if (this.pages.get(page.key) !== page || page.applied !== stamp) return;
      this.log.event("browser.route_failed", { detail: String(error) });
      this.update(page, { url, loading: false, failure: this.words()("native.browser.routeUnavailable") });
    });
  }

  private watch(page: Page): void {
    const contents = page.view.webContents;
    const history = () => ({ canGoBack: contents.navigationHistory.canGoBack(), canGoForward: contents.navigationHistory.canGoForward() });
    contents.on("did-start-loading", () => this.update(page, { loading: true, failure: null }));
    contents.on("did-stop-loading", () => this.update(page, { loading: false, ...history() }));
    contents.on("did-navigate", (_event, url) => {
      // A new document takes its host's zoom, and starts without the device size.
      this.update(page, { url: this.sourceAddress(page, url), zoom: contents.getZoomFactor(), ...history() });
      this.emulate(page, true);
      // The limits live in the page's renderer, so each committed document,
      // which may be a new renderer, gets them again.
      void contents.setVisualZoomLevelLimits(...PINCH_ZOOM_LIMITS).catch((error: unknown) => this.log.event("browser.pinch_zoom_failed", { detail: String(error) }));
    });
    contents.on("before-input-event", (event, input) => {
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
    contents.on("did-fail-load", (_event, code, description, url, isMainFrame) => {
      if (!isMainFrame || code === ERR_ABORTED) return;
      this.log.event("browser.load_failed", { code, description });
      this.update(page, { url: this.sourceAddress(page, url), loading: false, failure: this.words()("native.browser.loadFailed", { code }) });
    });
    contents.on("render-process-gone", (_event, details) => {
      if (this.cycleInput?.page === page) this.cancelCycle();
      this.log.event("browser.page_gone", { reason: details.reason });
      this.update(page, { loading: false, failure: this.words()("native.browser.stopped") });
    });
    contents.on("blur", () => this.log.event("browser.page_blur", { id: page.id, owed_page: this.returnTo === page, visible: page.visible, window_focused: this.window?.isFocused() ?? false, owed: this.returnTo !== null }));
    contents.on("focus", () => {
      this.log.event("browser.page_focus", { id: page.id, owed_page: this.returnTo === page, visible: page.visible, window_focused: this.window?.isFocused() ?? false, owed: this.returnTo !== null });
      // Another page holding the keyboard settles the debt: it is not the page the hold began on.
      if (this.returnTo && this.returnTo !== page) this.returnTo = null;
      if (page.visible) this.emit({ kind: "focus", workspace: page.workspace, id: page.id });
    });
    this.guard(page, contents);
  }

  /** Where the contents of `page`, or of a popup it owns, may go and what may open from them. */
  private guard(page: Page, contents: WebContents): void {
    contents.on("did-frame-navigate", (_event, url) => {
      if (!isFileAddress(url) || page.cdpFileFrame) return;
      page.cdpFileFrame = true;
      this.publishCdp({ contentsId: page.view.webContents.id, reason: "replaced" });
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
      if (page.route && isFileAddress(page.route.source_url) && remoteRequest(page.route, url).cancel) {
        event.preventDefault();
        this.log.event("browser.navigation_refused", { reason: "remote_file_boundary" });
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
    if (page.route && isFileAddress(page.route.source_url) && remoteRequest(page.route, url).cancel) {
      this.log.event("browser.window_open_refused", { reason: "remote_file_boundary" });
      return { action: "deny" };
    }
    if (!loadable(url)) {
      // A frame's window.open names its frame only through the referrer, when its policy leaves one.
      if (appScheme(url)) this.openInApp(page, contents, url, referrer || undefined);
      else this.log.event("browser.window_open_refused", { protocol: protocolOf(url) });
      return { action: "deny" };
    }
    if (popup) return this.popup(page, contents);
    if (url !== "about:blank") this.emit({ kind: "open", workspace: page.workspace, id: page.id, url: this.sourceAddress(page, url) });
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
    return {
      action: "allow",
      createWindow: (options) => {
        const bounds = parent.getBounds();
        const window = new BrowserWindow({
          ...popupBounds({ width: options.width, height: options.height }, bounds, screen.getDisplayMatching(bounds).workArea),
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

  /** A popup takes the keyboard only from a page that held it, so a page cannot pull focus from the operator's typing. */
  private adopt(owner: Page, window: BrowserWindow, focus: boolean): void {
    const id = window.webContents.id;
    this.popups.set(id, { window, owner });
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
    if (contents.id === page.view.webContents.id) return page.visible;
    return this.popups.get(contents.id)?.window.isVisible() ?? false;
  }

  /** The page a request or a question comes from: a page's own contents, or a popup it owns. */
  private ownerOf(contentsId: number | undefined): Page | undefined {
    if (contentsId === undefined) return undefined;
    for (const page of this.pages.values()) if (page.view.webContents.id === contentsId) return page;
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
    this.emit({ kind: "cycle-input", cycleId: cycle.cycleId, workspace: cycle.page.workspace, id: cycle.page.id, type: input.type as "keyDown" | "keyUp", key: input.key, code: input.code, control: input.control, alt: input.alt, meta: input.meta, shift: input.shift });
    // The overlay can be outside the originating page. Transfer its native
    // response at the start, while its logical owner stays frozen in the shell.
    if (starts && this.window?.isFocused()) this.window.webContents.focus();
    return true;
  }

  private update(page: Page, change: Partial<BrowserPageState>): void {
    page.state = { ...page.state, ...change };
    if (page.report) return;
    page.report = setTimeout(() => {
      page.report = null;
      if (this.pages.get(page.key) !== page) return;
      this.emit({ kind: "state", workspace: page.workspace, id: page.id, load: page.applied, state: page.state });
      this.publishCdp();
    }, REPORT_COALESCE_MS);
  }

  private sourceAddress(page: Page, raw: string): string {
    const route = page.route;
    if (!route || route.url === route.source_url) return raw;
    if (raw === route.url) return route.source_url;
    try {
      const address = new URL(raw);
      const local = new URL(route.url);
      const source = new URL(route.source_url);
      if (address.origin !== local.origin) return raw;
      if (source.protocol === "file:") {
        const workspace = page.workspace.split("\u0000");
        const prefix = `/${local.pathname.split("/")[1]}/`;
        if (workspace.length !== 2 || !address.pathname.startsWith(prefix)) return raw;
        return `${pathToFileURL(path.join(workspace[1]!, decodeURIComponent(address.pathname.slice(prefix.length)))).href}${address.search}${address.hash}`;
      }
      address.host = source.host;
      address.protocol = source.protocol;
      return address.href;
    } catch { return raw; }
  }

  private emit(event: BrowserHostEvent): void {
    const contents = this.window?.webContents;
    if (contents && !contents.isDestroyed()) contents.send(BROWSER_EVENT_CHANNEL, event);
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
    if (!visible && (keyboard || this.cycleInput?.page === page) && this.window?.isFocused()) this.window.webContents.focus();
  }

  private hideAll(): void {
    for (const page of this.pages.values()) this.show(page, false);
  }

  private destroy(page: Page, reason: CdpRetirement["reason"]): void {
    if (reason === "evicted") this.emit({ kind: "gone", workspace: page.workspace, id: page.id, load: page.applied, url: page.state.url });
    if (this.cycleInput?.page === page) this.cancelCycle();
    if (this.returnTo === page) this.returnTo = null;
    this.pages.delete(page.key);
    this.publishCdp({ contentsId: page.view.webContents.id, reason });
    this.release(page.workspace, page.id, page.applied);
    if (page.report) clearTimeout(page.report);
    const window = this.window;
    if (window && !window.isDestroyed()) window.contentView.removeChildView(page.view);
    // Closing the contents ends the page's renderer process; a view left
    // alone keeps it running after the view is gone.
    if (!page.view.webContents.isDestroyed()) page.view.webContents.close();
    this.log.event("browser.view_closed", { reason, live: this.pages.size });
  }
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

function protocolOf(url: string): string {
  try {
    return new URL(url).protocol;
  } catch {
    return "unparsable";
  }
}

function menuHas(items: readonly MenuItem[], accelerator: string): boolean {
  return items.some((item) => item.accelerator === accelerator || (item.submenu ? menuHas(item.submenu.items, accelerator) : false));
}

function isFileAddress(url: string): boolean {
  return protocolOf(url) === "file:";
}

/** An outside file's session: in memory, its grant's alone (PRD outside-path-in-hide B4). */
export function externalPartition(grant: string): string {
  return `hide-browser-external-${grant}`;
}
