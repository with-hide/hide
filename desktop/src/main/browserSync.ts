// What the shell may ask of the browser views, checked before anything is
// placed or loaded. The shell is a page served over loopback; its messages
// are input like any other, so every field is bounded here and a message
// that fails is dropped whole (issue 155).

import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import type { Parameters as EmulationParameters } from "electron";
import type { BrowserCommand, BrowserDevicePlacement, BrowserPlacement, BrowserRect, BrowserSync } from "../../../web/src/host";
import { isLocalNetworkHost, isLoopbackHost } from "../../../web/src/loopbackHost";

export { isLocalNetworkHost };

/** A Workspace holds at most this many displays (`MAX_VIEW_DISPLAYS` is 32);
 * the bound is kept wider than the core's. */
export const MAX_SYNCED_DISPLAYS = 256;
/** Displays across at most 256 Workspaces. */
export const MAX_RETAINED_DISPLAYS = 256 * MAX_SYNCED_DISPLAYS;
/** Positive areas across at most 256 Workspaces: every area but an empty front
 * one holds a display, so a Workspace has at most one per display plus that empty one. */
export const MAX_AUTHORIZED_SCOPES = 256 * (MAX_SYNCED_DISPLAYS + 1);
/** Live pages at once; a hidden one past this is closed and loads again when shown. */
export const MAX_LIVE_VIEWS = 12;
/** How long a hidden page stays alive unseen and idle before it is closed, to load again when shown or driven. */
export const IDLE_UNLOAD_MS = 12 * 60 * 60 * 1000;
/** How often the window's pages are checked against `IDLE_UNLOAD_MS`; the one work an idle screen adds. */
export const IDLE_CHECK_MS = 60_000;
/** Popup windows open at once across every page; a page asking for one more is refused. */
export const MAX_POPUPS = 4;

/**
 * What one other computer's screen may hold of this window's pages: a
 * page there is that computer's to place, so each repeatable thing it asks
 * for is bounded on its own, apart from this window's `MAX_LIVE_VIEWS`.
 */
export const REMOTE_BOUNDS = {
  /** Live pages at once; a placement past it gets no page and says why. */
  pages: 4,
  /** Pages made within `creationWindowMs`; past it a display waits. */
  creations: 8,
  creationWindowMs: 10_000,
  /** Popups open at once, within `MAX_POPUPS`. */
  popups: 2,
  /** Placements and retained rows one sync may name. */
  placements: 64,
  retained: 1024,
  /** The least time between two syncs applied; the latest waiting wins. */
  syncEveryMs: 16,
  /** Stills a second, one at a time. */
  capturesPerSecond: 10,
  /** Toolbar commands a second. */
  commandsPerSecond: 20,
} as const;

/** How long after the operator's press a page may open a window or this Mac's browser. */
export const PRESS_WITHIN_MS = 1_000;
const MAX_TEXT = 8192;
/** Outside-file grants one sync may name: the host holds at most 32 (`MAX_EXTERNAL_GRANTS`), and the shell's opens on their way add a few. */
const MAX_EXTERNAL_LISTED = 256;
const MAX_EXTENT = 100_000;
const COMMANDS: ReadonlySet<string> = new Set<BrowserCommand>(["back", "forward", "reload", "stop", "focus", "zoom_in", "zoom_out", "zoom_reset"]);
/** The widest viewport a device size may ask for, in CSS pixels. */
const MAX_DEVICE_WIDTH = 4096;
const LOADABLE_PROTOCOLS: ReadonlySet<string> = new Set(["http:", "https:", "file:"]);
/** Schemes Chromium answers itself (with every `chrome` one): a page never reaches them, and they are never another app's. */
const BROWSER_PROTOCOLS: ReadonlySet<string> = new Set([
  "about:", "blob:", "data:", "devtools:", "filesystem:", "isolated-app:", "javascript:", "view-source:", "ws:", "wss:",
]);
/**
 * Schemes macOS hands to a file share, a shell, a script or a remote session
 * (with every `x-apple` one): one click on a question the operator did not
 * expect must not mount a share or run anything, so a page never offers them.
 */
const SYSTEM_PROTOCOLS: ReadonlySet<string> = new Set([
  "afp:", "applescript:", "cifs:", "disk:", "disks:", "ftp:", "ftps:", "gopher:", "hcp:", "ms-help:", "news:", "nfs:", "nntp:", "rdp:", "rlogin:", "screens:", "sftp:", "shell:", "shortcuts:", "smb:", "snews:", "ssh:", "telnet:", "vbscript:", "vnc:", "x-man-page:",
]);
/** A popup's smallest and default content size, in points. */
const POPUP_MIN = { width: 320, height: 240 };
const POPUP_DEFAULT = { width: 500, height: 600 };

function text(value: unknown): value is string {
  return typeof value === "string" && value.length > 0 && value.length <= MAX_TEXT;
}

function identity(value: unknown): value is string {
  return text(value) && !value.includes("\u0000") && !value.includes("\u0001");
}

function workspaceIdentity(value: unknown): value is string {
  if (!text(value) || value.includes("\u0001")) return false;
  const at = value.indexOf("\u0000");
  return at > 0 && at < value.length - 1 && value.indexOf("\u0000", at + 1) === -1;
}

/** The shape of a grant id `ExternalFiles` mints: 128 random bits in lower-case hex. */
function externalGrant(value: unknown): value is string {
  return typeof value === "string" && /^[0-9a-f]{32}$/.test(value);
}

function rect(value: unknown): BrowserRect | null | undefined {
  if (value === null) return null;
  if (typeof value !== "object") return undefined;
  const { x, y, width, height } = value as Record<string, unknown>;
  const numbers = [x, y, width, height];
  if (!numbers.every((n) => typeof n === "number" && Number.isFinite(n) && Math.abs(n) <= MAX_EXTENT)) return undefined;
  if ((width as number) < 0 || (height as number) < 0) return undefined;
  return { x: x as number, y: y as number, width: width as number, height: height as number };
}

/** A display's device size: null or absent fits its slot; undefined is out of shape. */
function device(value: unknown): BrowserDevicePlacement | null | undefined {
  if (value === undefined || value === null) return null;
  if (typeof value !== "object") return undefined;
  const { width, mobile } = value as Record<string, unknown>;
  if (typeof width !== "number" || !Number.isSafeInteger(width) || width < 1 || width > MAX_DEVICE_WIDTH || typeof mobile !== "boolean") return undefined;
  return { width, mobile };
}

/** A sync message, or null when any part of it is out of shape. */
export function parseSync(value: unknown): BrowserSync | null {
  if (typeof value !== "object" || value === null) return null;
  const { workspace, displays, retained, attachment_epoch, authorized_scopes, node, external_grants, inventoried } = value as Record<string, unknown>;
  if (inventoried !== undefined && inventoried !== true) return null;
  if (external_grants !== undefined && (!Array.isArray(external_grants) || external_grants.length > MAX_EXTERNAL_LISTED || !external_grants.every(externalGrant))) return null;
  if (attachment_epoch !== undefined && (typeof attachment_epoch !== "string" || attachment_epoch.length === 0 || attachment_epoch.length > 64 || /[^A-Za-z0-9-]/.test(attachment_epoch))) return null;
  if (workspace !== null && !workspaceIdentity(workspace)) return null;
  if (node !== undefined && !identity(node)) return null;
  if (!Array.isArray(displays) || displays.length > MAX_SYNCED_DISPLAYS) return null;
  if (!Array.isArray(retained) || retained.length > MAX_RETAINED_DISPLAYS) return null;
  const scopes: NonNullable<BrowserSync["authorized_scopes"]> = [];
  if (authorized_scopes !== undefined) {
    if (!Array.isArray(authorized_scopes) || authorized_scopes.length > MAX_AUTHORIZED_SCOPES) return null;
    const scopeKeys = new Set<string>();
    for (const entry of authorized_scopes as unknown[]) {
      if (typeof entry !== "object" || entry === null) return null;
      const { workspace: owner, area_id, incarnation } = entry as Record<string, unknown>;
      if (!workspaceIdentity(owner) || !identity(area_id) || typeof incarnation !== "number" || !Number.isSafeInteger(incarnation) || incarnation < 0) return null;
      const key = viewKey(owner, area_id);
      if (scopeKeys.has(key)) return null;
      scopeKeys.add(key);
      scopes.push({ workspace: owner, area_id, incarnation });
    }
  }
  const owned: BrowserSync["retained"] = [];
  const ownedAreas = new Map<string, string>();
  for (const entry of retained as unknown[]) {
    if (typeof entry !== "object" || entry === null) return null;
    const { workspace: owner, id, area_id, url, load } = entry as Record<string, unknown>;
    if (!workspaceIdentity(owner) || !identity(id) || !identity(area_id)) return null;
    // A web page names its address and load together, or neither (an outside file).
    if ((url === undefined) !== (load === undefined)) return null;
    if (url !== undefined && (!text(url) || typeof load !== "number" || !Number.isSafeInteger(load) || load < 0)) return null;
    const key = viewKey(owner, id);
    if (ownedAreas.has(key)) return null;
    ownedAreas.set(key, area_id);
    owned.push({ workspace: owner, id, area_id, ...(url === undefined ? {} : { url: url as string, load: load as number }) });
  }
  const parsed: BrowserPlacement[] = [];
  const ids = new Set<string>();
  for (const entry of displays as unknown[]) {
    if (typeof entry !== "object" || entry === null) return null;
    const { id, area_id, url, load, visible, grant } = entry as Record<string, unknown>;
    const placed = rect((entry as Record<string, unknown>).rect);
    const size = device((entry as Record<string, unknown>).device);
    // An outside file names its grant and no address (PRD outside-path-in-hide).
    const address = grant === undefined ? text(url) : externalGrant(grant) && url === "";
    if (!identity(id) || !identity(area_id) || ids.has(id) || !address || typeof load !== "number" || !Number.isSafeInteger(load) || load < 0) return null;
    if (placed === undefined || size === undefined || typeof visible !== "boolean") return null;
    ids.add(id);
    parsed.push({ id, area_id, url: url as string, load, rect: placed, visible, ...(size ? { device: size } : {}), ...(grant === undefined ? {} : { grant: grant as string }) });
  }
  if (workspace === null && parsed.length > 0) return null;
  if (workspace && parsed.some((display) => ownedAreas.get(viewKey(workspace, display.id)) !== display.area_id)) return null;
  return {
    workspace: workspace as string | null,
    displays: parsed,
    retained: owned,
    ...(attachment_epoch === undefined ? {} : { attachment_epoch }),
    ...(authorized_scopes === undefined ? {} : { authorized_scopes: scopes }),
    ...(node === undefined ? {} : { node: node as string }),
    ...(external_grants === undefined ? {} : { external_grants: external_grants as string[] }),
    ...(inventoried ? { inventoried: true as const } : {}),
  };
}

/**
 * A sync from another computer's screen: within that screen's
 * bounds and with nothing that grants this window's authority. A display
 * that names an outside file's grant drops the whole sync; the CDP scopes,
 * the attachment epoch and the outside grants it may name are dropped, since
 * no page of that screen enters CDP or reads this Mac's files.
 */
export function parseRemoteSync(value: unknown): BrowserSync | null {
  if (typeof value !== "object" || value === null) return null;
  const { displays, retained } = value as Record<string, unknown>;
  if (!Array.isArray(displays) || displays.length > REMOTE_BOUNDS.placements) return null;
  if (!Array.isArray(retained) || retained.length > REMOTE_BOUNDS.retained) return null;
  if (displays.some((display) => typeof display !== "object" || display === null || "grant" in display)) return null;
  const sync = parseSync({ ...(value as Record<string, unknown>), authorized_scopes: undefined, attachment_epoch: undefined, external_grants: undefined });
  if (!sync) return null;
  return { workspace: sync.workspace, displays: sync.displays, retained: sync.retained, ...(sync.node ? { node: sync.node } : {}), ...(sync.inventoried ? { inventoried: true as const } : {}) };
}

/** A display named by the shell for a still or a command. */
export function parseTarget(value: unknown): { workspace: string; id: string } | null {
  if (typeof value !== "object" || value === null) return null;
  const { workspace, id } = value as Record<string, unknown>;
  return text(workspace) && text(id) ? { workspace, id } : null;
}

export function parseCommand(value: unknown): BrowserCommand | null {
  return typeof value === "string" && COMMANDS.has(value) ? (value as BrowserCommand) : null;
}

/** Whether a view may load `url`: the web, a local file, or a blank page. */
export function loadable(url: string): boolean {
  if (url === "about:blank") return true;
  try {
    return LOADABLE_PROTOCOLS.has(new URL(url).protocol);
  } catch {
    return false;
  }
}

/**
 * Whether a page's new window is a sized popup that keeps its opener: Chromium's
 * `new-window` disposition with window features, the way a sign-in button opens
 * one. A shift-click on a link is `new-window` too, with no features; that is a
 * tab, like `target=_blank`.
 */
export function isPopup(disposition: string, features: string): boolean {
  return disposition === "new-window" && features.trim() !== "";
}

/**
 * The scheme of a link a page may hand to another app (`slack:`, `zoommtg:`,
 * `mailto:`), or null for one a view loads, one Chromium answers itself, one
 * that reaches the file system, a shell or a remote session, or an address
 * that does not parse.
 */
export function appScheme(url: string): string | null {
  let protocol: string;
  try {
    protocol = new URL(url).protocol;
  } catch {
    return null;
  }
  if (LOADABLE_PROTOCOLS.has(protocol) || BROWSER_PROTOCOLS.has(protocol) || SYSTEM_PROTOCOLS.has(protocol)) return null;
  return protocol.startsWith("chrome") || protocol.startsWith("x-apple") ? null : protocol;
}

type Bounds = { x: number; y: number; width: number; height: number };

/**
 * Where a popup sits: the content size the page asked for, held between a
 * usable minimum and the work area, centred over hide's window and kept on
 * the work area. Nothing else a page writes in its window features reaches
 * the window.
 */
export function popupBounds(asked: { width?: number; height?: number }, parent: Bounds, workArea: Bounds): Bounds {
  const size = (value: number | undefined, fallback: number, min: number, max: number) =>
    Math.round(Math.min(Math.max(Number.isFinite(value) && value! > 0 ? value! : fallback, min), Math.max(min, max)));
  const width = size(asked.width, POPUP_DEFAULT.width, POPUP_MIN.width, workArea.width);
  const height = size(asked.height, POPUP_DEFAULT.height, POPUP_MIN.height, workArea.height);
  const within = (start: number, length: number, areaStart: number, areaLength: number) =>
    Math.round(Math.min(Math.max(start, areaStart), areaStart + Math.max(0, areaLength - length)));
  return {
    x: within(parent.x + (parent.width - width) / 2, width, workArea.x, workArea.width),
    y: within(parent.y + (parent.height - height) / 2, height, workArea.y, workArea.height),
    width,
    height,
  };
}

/**
 * Web logins are shared across Workspaces; file previews and each remote
 * device's loopback stay separate. Only the core's own node (`node`, null
 * until the shell names it) loads this computer's loopback in the shared one.
 */
export function browserPartition(workspace: string, url: string, node: string | null): string {
  const separator = workspace.indexOf("\u0000");
  if (separator < 1) throw new Error("Browser Workspace key is missing its device");
  const device = workspace.slice(0, separator);
  const address = new URL(url);
  if (address.protocol === "file:") {
    return `persist:hide-browser-file-${createHash("sha256").update(workspace).digest("hex").slice(0, 32)}`;
  }
  if (device !== node && isLoopbackHost(address.hostname)) {
    return `persist:hide-browser-loopback-${createHash("sha256").update(device).digest("hex").slice(0, 32)}`;
  }
  return "persist:hide-browser-web";
}

/**
 * The login storage of the pages another computer's screen shows here: one
 * persistent partition per machine, the key it was paired under and the
 * pairing itself (the daemon's name for the grant that machine gave),
 * shared with no page of this window, of another machine, of that machine
 * under another key, or of an earlier pairing of it.
 */
export function remotePagePartition(device: string, key: string, pairing: string): string {
  return `${REMOTE_PAGE_PARTITION}${pairHash(device, key, pairing)}`;
}

/** The name every remote screen's login storage starts with (`remotePagePartition`). */
export const REMOTE_PAGE_PARTITION = "persist:hide-remote-page-";
/** The name every remote screen's file page storage starts with (`remoteFilePartition`). */
export const REMOTE_FILE_PARTITION = "hide-remote-file-";

/**
 * The storage of the file pages another computer's screen shows here (that
 * computer's checkout HTML, served from its own route): one in-memory
 * partition per machine, key and pairing, apart from that screen's logins
 * (`remotePagePartition`), so an HTML file of a checkout reads none of them,
 * and gone with the app.
 */
export function remoteFilePartition(device: string, key: string, pairing: string): string {
  return `${REMOTE_FILE_PARTITION}${pairHash(device, key, pairing)}`;
}

function pairHash(device: string, key: string, pairing: string): string {
  return createHash("sha256").update(`${device}\u0000${key}\u0000${pairing}`).digest("hex").slice(0, 32);
}

/**
 * Whether a route another computer answers for a page of its screen loads
 * from where it may (TF1): a web page from its own address, and a file page
 * only from that computer's own loopback over plain HTTP, a numeric address
 * with a port, which the screen's proxy carries to that computer; never a
 * file, a name, or anything that reaches this computer.
 */
export function remoteRouteAddress(route: PageRoute): boolean {
  if (protocolOf(route.source_url) !== "file:") return route.url === route.source_url;
  try {
    const address = new URL(route.url);
    const loopback = /^127(?:\.\d{1,3}){3}$/.test(address.hostname) || address.hostname === "[::1]";
    return address.protocol === "http:" && loopback && address.port !== "" && address.username === "" && address.password === "";
  } catch {
    return false;
  }
}

/**
 * Where a page another computer's screen placed draws in this window:
 * its rect in that screen's page, scaled by the screen's zoom, moved to the
 * screen's place in the window and cut to it, or null when nothing of it is
 * inside, so no page of that computer ever covers this window's own rail.
 */
export function placeIn(placed: BrowserRect, zoom: number, area: BrowserRect): BrowserRect | null {
  const inner = toBounds(placed, zoom);
  const x = Math.max(area.x, area.x + inner.x);
  const y = Math.max(area.y, area.y + inner.y);
  const right = Math.min(area.x + area.width, area.x + inner.x + inner.width);
  const bottom = Math.min(area.y + area.height, area.y + inner.y + inner.height);
  return right > x && bottom > y ? { x, y, width: right - x, height: bottom - y } : null;
}

/**
 * Whether a page of another computer's screen may reach `raw`: a page
 * reaches that computer's loopback and network only from a document that is
 * itself there. Chromium cannot tell a public page from a private one behind
 * a loopback proxy, so a subresource, fetch or socket to a local-network
 * host is refused unless `top`, the document that asks, is on one; a request
 * with no known document (a service worker) counts as a public page's. A
 * top-level navigation decides its own document and is allowed.
 */
export function localNetworkReach(raw: string, top: string | null, mainFrame: boolean): boolean {
  if (mainFrame) return true;
  let target: URL;
  try { target = new URL(raw); } catch { return false; }
  if (!["http:", "https:", "ws:", "wss:"].includes(target.protocol)) return true;
  if (!isLocalNetworkHost(target.hostname)) return true;
  if (!top) return false;
  try {
    const document = new URL(top);
    return (document.protocol === "http:" || document.protocol === "https:") && isLocalNetworkHost(document.hostname);
  } catch {
    return false;
  }
}

/** A page's address (`source_url`, the one the core keeps) and the address this Mac loads it from (`url`). */
export type PageRoute = { url: string; source_url: string };

/**
 * Whether a page reaches this Mac through a route from another machine: hided
 * answers a loopback route of this Mac's for a connected device's file or
 * loopback page, or for the core machine's loopback page on a node's window,
 * and the page's own address for a page this Mac loads as it is (a web page,
 * a local file page, an outside file). The one test of it; every caller asks
 * here rather than reading the two addresses itself.
 */
export function routed(route: PageRoute): boolean {
  return route.url !== route.source_url;
}

/**
 * Where a file page's navigation or new tab to `raw` goes (null for a page
 * that is not a file page). A routed file page, a device's, stays on its own
 * route and anything else is `refuse`d; a local one hands an `http` or `https`
 * address to a browser display beside it (`beside`), since a web page does not
 * load in a file page's partition, and `follow`s the rest, which the file and
 * app link rules then judge.
 */
export function fileLink(route: PageRoute | null, raw: string): "follow" | "beside" | "refuse" | null {
  if (!route || protocolOf(route.source_url) !== "file:") return null;
  if (routed(route)) return remoteRequest(route, raw).cancel ? "refuse" : "follow";
  const target = protocolOf(raw);
  return target === "http:" || target === "https:" ? "beside" : "follow";
}

/**
 * The `file:` address of `relative` (a path of `/`-separated names) under
 * `root`, a checkout's root in the wire spelling of the machine that holds
 * it (`/Users/me/repo`, or `C:/repo` on Windows), with `search` and `hash` as
 * a URL spells them; null when either names no file under the root. Built as
 * URL text, each name escaped as a URL path needs, never through this
 * machine's own path rules, since the file may be on another system.
 */
export function wireFileAddress(root: string, relative: string, search = "", hash = ""): string | null {
  const names = `${root.replace(/^\/+/, "").replace(/\/+$/, "")}/${relative}`.split("/");
  if (names.some((name) => name === "" || name === "." || name === "..")) return null;
  return `file:///${names.map((name) => encodeURIComponent(name).replace(/%3A/gi, ":")).join("/")}${search}${hash}`;
}

/**
 * Whether two `file:` addresses name the same file, read as URLs alone (the
 * same host and decoded path; their queries and fragments may differ),
 * never through this machine's own path rules, since the file may be on
 * another system.
 */
export function sameFileAddress(a: string, b: string): boolean {
  try {
    const one = new URL(a);
    const other = new URL(b);
    return one.protocol === "file:" && other.protocol === "file:" && one.host === other.host && decodeURIComponent(one.pathname) === decodeURIComponent(other.pathname);
  } catch {
    return false;
  }
}

/** An address's scheme as `URL` spells it (`https:`), or `unparsable`. */
export function protocolOf(url: string): string {
  try {
    return new URL(url).protocol;
  } catch {
    return "unparsable";
  }
}

/** Route a remote page's absolute loopback requests through its owned View.
 * An unsupported local address is refused instead of reaching this Mac. */
export function remoteRequest(route: PageRoute, raw: string): { redirectURL?: string; cancel?: boolean } {
  try {
    const address = new URL(raw);
    const local = new URL(route.url);
    const source = new URL(route.source_url);
    if (source.protocol === "file:") return address.origin === local.origin && address.protocol === local.protocol ? {} : { cancel: true };
    const loopback = isLoopbackHost(address.hostname);
    if (!routed(route)) return loopback ? { cancel: true } : {};
    if (address.origin === local.origin || (address.protocol === "ws:" && local.protocol === "http:" && address.host === local.host) || (address.protocol === "wss:" && local.protocol === "https:" && address.host === local.host)) return {};
    if (!loopback) return {};
    const sourcePort = source.port || (source.protocol === "https:" ? "443" : "80");
    const addressPort = address.port || ((address.protocol === "https:" || address.protocol === "wss:") ? "443" : "80");
    const matchingScheme = address.protocol === source.protocol || (address.protocol === "ws:" && source.protocol === "http:") || (address.protocol === "wss:" && source.protocol === "https:");
    if (!matchingScheme || addressPort !== sourcePort) return { cancel: true };
    address.host = local.host;
    if (address.protocol === "http:" || address.protocol === "https:") address.protocol = local.protocol;
    return { redirectURL: address.href };
  } catch {
    return { cancel: true };
  }
}

export type LocalPageVerdict = { cancel: false } | { cancel: true; boundary: "file" | "network" } | { resolve: string };

/**
 * What a local HTML page (a `file:` page the operator or an agent opened) may
 * load: what a browser allows a local page, and no more. Its own load is the
 * display's address; its files, scripts, styles, images and frames load from
 * its checkout only; a script reads no file (no `fetch` or XHR of one); and it
 * reaches no address of this machine or a private network, so Hide's daemon
 * and the operator's dev servers stay out of reach. A public name is
 * `resolve`d by the caller before it is allowed, since a name can answer with
 * a private address.
 */
export function localPageRequest(raw: string, resourceType: string, insideCheckout: (path: string) => boolean): LocalPageVerdict {
  let url: URL;
  try { url = new URL(raw); } catch { return { cancel: true, boundary: "network" }; }
  switch (url.protocol) {
    case "file:": {
      if (resourceType === "mainFrame") return { cancel: false };
      if (resourceType === "xhr" || resourceType === "webSocket" || resourceType === "ping" || resourceType === "cspReport" || resourceType === "other") return { cancel: true, boundary: "file" };
      let file: string;
      try { file = fileURLToPath(url); } catch { return { cancel: true, boundary: "file" }; }
      return insideCheckout(file) ? { cancel: false } : { cancel: true, boundary: "file" };
    }
    case "http:": case "https:": case "ws:": case "wss:":
      if (isLocalNetworkHost(url.hostname)) return { cancel: true, boundary: "network" };
      return /^[\d.]+$|^\[/.test(url.hostname) ? { cancel: false } : { resolve: url.hostname };
    case "data:": case "blob:": case "about:":
      return { cancel: false };
    default:
      return { cancel: true, boundary: "network" };
  }
}

/** The one key a page lives under: display ids repeat across Workspaces. */
export function viewKey(workspace: string, id: string): string {
  return `${workspace}\u0001${id}`;
}

export type LiveView = { key: string; visible: boolean; shownAt: number };

/** The width a page no slot shows lays out at: the lane's width for a page (PRD lane-desk-pair-view B32). */
export const OFF_SCREEN_WIDTH = 900;

/**
 * The bounds of a page loaded with no slot on screen: the lane's width for a
 * page, or the window's when narrower, at the window's height, so an agent's
 * snapshot and clicks meet the layout the operator would see.
 */
export function offScreenBounds(content: { width: number; height: number }): BrowserRect {
  return { x: 0, y: 0, width: Math.max(1, Math.min(OFF_SCREEN_WIDTH, Math.round(content.width))), height: Math.max(1, Math.round(content.height)) };
}

/**
 * The hidden pages to close so no more than `cap` stay alive, least recently
 * shown first. A page on screen is never closed; there are at most as many
 * of those as View areas.
 */
export function overCap(views: readonly LiveView[], cap: number): string[] {
  const excess = views.length - cap;
  if (excess <= 0) return [];
  return views
    .filter((view) => !view.visible)
    .sort((a, b) => a.shownAt - b.shownAt)
    .slice(0, excess)
    .map((view) => view.key);
}

/** A live page as the idle check sees it: `held` while it is on screen or doing something that would break (playing, downloading, driven, a popup open). */
export type IdleView = { key: string; held: boolean; idleSince: number };

/**
 * The pages to close because they have been hidden and idle for `bound`
 * since `idleSince`; a held page is never one, whatever its age.
 */
export function idleExpired(views: readonly IdleView[], now: number, bound: number): string[] {
  return views.filter((view) => !view.held && now - view.idleSince >= bound).map((view) => view.key);
}

/**
 * What a page at a device size is emulated with (PRD browser-width-zoom
 * D-12): a viewport `device.width` CSS pixels wide whatever the page's zoom,
 * as tall as its rectangle at the scale that fits that width into it, drawn
 * scaled to fill `bounds` (whole window points). A desktop size keeps the
 * page's zoom, so its viewport is the zoomed width; a mobile one lays out at
 * the device's own width, which Chromium does not zoom.
 */
export function deviceEmulation(device: BrowserDevicePlacement, bounds: BrowserRect, zoom: number): EmulationParameters {
  const factor = device.mobile || !Number.isFinite(zoom) || zoom <= 0 ? 1 : zoom;
  const fit = bounds.width > 0 ? bounds.width / device.width : 1;
  const view = { width: Math.round(device.width * factor), height: Math.max(1, Math.round((bounds.height / fit) * factor)) };
  return {
    screenPosition: device.mobile ? "mobile" : "desktop",
    screenSize: view,
    viewPosition: { x: 0, y: 0 },
    deviceScaleFactor: 0,
    viewSize: view,
    scale: bounds.width > 0 ? bounds.width / view.width : 1,
  };
}

/**
 * The layout of a page no slot has shown since its contents were made: a
 * desktop viewport of `bounds`' size at scale 1. Chromium gives a page it
 * never showed no viewport, so without this an agent's snapshot reads no
 * layout and its clicks land outside the page (PRD lane-desk-pair-view B32).
 */
export function offScreenEmulation(bounds: BrowserRect): EmulationParameters {
  const view = { width: Math.max(1, bounds.width), height: Math.max(1, bounds.height) };
  return { screenPosition: "desktop", screenSize: view, viewPosition: { x: 0, y: 0 }, deviceScaleFactor: 0, viewSize: view, scale: 1 };
}

/** A rect in the shell's CSS pixels as whole window points. */
export function toBounds(placed: BrowserRect, zoom: number): BrowserRect {
  const scale = Number.isFinite(zoom) && zoom > 0 ? zoom : 1;
  const x = Math.round(placed.x * scale);
  const y = Math.round(placed.y * scale);
  return { x, y, width: Math.max(0, Math.round((placed.x + placed.width) * scale) - x), height: Math.max(0, Math.round((placed.y + placed.height) * scale) - y) };
}
