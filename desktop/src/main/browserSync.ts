// What the shell may ask of the browser views, checked before anything is
// placed or loaded. The shell is a page served over loopback; its messages
// are input like any other, so every field is bounded here and a message
// that fails is dropped whole (issue 155).

import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import type { Parameters as EmulationParameters } from "electron";
import type { BrowserCommand, BrowserDevicePlacement, BrowserPlacement, BrowserRect, BrowserSync } from "../../../web/src/host";

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
/** Popup windows open at once across every page; a page asking for one more is refused. */
export const MAX_POPUPS = 4;
const MAX_TEXT = 8192;
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
  const { workspace, displays, retained, attachment_epoch, authorized_scopes, node } = value as Record<string, unknown>;
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
    const { workspace: owner, id, area_id } = entry as Record<string, unknown>;
    if (!workspaceIdentity(owner) || !identity(id) || !identity(area_id)) return null;
    const key = viewKey(owner, id);
    if (ownedAreas.has(key)) return null;
    ownedAreas.set(key, area_id);
    owned.push({ workspace: owner, id, area_id });
  }
  const parsed: BrowserPlacement[] = [];
  const ids = new Set<string>();
  for (const entry of displays as unknown[]) {
    if (typeof entry !== "object" || entry === null) return null;
    const { id, area_id, url, load, visible } = entry as Record<string, unknown>;
    const placed = rect((entry as Record<string, unknown>).rect);
    const size = device((entry as Record<string, unknown>).device);
    if (!identity(id) || !identity(area_id) || ids.has(id) || !text(url) || typeof load !== "number" || !Number.isSafeInteger(load) || load < 0) return null;
    if (placed === undefined || size === undefined || typeof visible !== "boolean") return null;
    ids.add(id);
    parsed.push({ id, area_id, url, load, rect: placed, visible, ...(size ? { device: size } : {}) });
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
  };
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

function isLoopbackHost(hostname: string): boolean {
  const host = hostname.toLowerCase().replace(/\.$/, "");
  const mapped = /^\[::(?:(?:ffff:)?([0-9a-f]{1,4}):([0-9a-f]{1,4}))\]$/.exec(host);
  const embeddedLoopback = mapped ? Number.parseInt(mapped[1]!, 16) >> 8 === 127 : false;
  return host === "localhost" || host === "0.0.0.0" || host === "[::1]" || /^127\./.test(host) || embeddedLoopback;
}

/** Route a remote page's absolute loopback requests through its owned View.
 * An unsupported local address is refused instead of reaching this Mac. */
export function remoteRequest(route: { url: string; source_url: string }, raw: string): { redirectURL?: string; cancel?: boolean } {
  try {
    const address = new URL(raw);
    const local = new URL(route.url);
    const source = new URL(route.source_url);
    if (source.protocol === "file:") return address.origin === local.origin && address.protocol === local.protocol ? {} : { cancel: true };
    const loopback = isLoopbackHost(address.hostname);
    if (route.url === route.source_url) return loopback ? { cancel: true } : {};
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

/**
 * Whether `hostname` (as a URL spells it, an IPv6 literal in brackets) names
 * this machine or a private network: loopback, link-local, private and
 * shared ranges, and the names a resolver answers from the local network
 * rather than the internet (`localhost`, `.local`, and a name with no dot).
 */
export function isLocalNetworkHost(hostname: string): boolean {
  const host = hostname.toLowerCase().replace(/\.$/, "");
  if (host === "" || host === "localhost" || /\.(localhost|local|localdomain|internal|lan|home\.arpa|intranet)$/.test(host)) return true;
  const v4 = /^(\d+)\.(\d+)\.(\d+)\.(\d+)$/.exec(host);
  if (v4) return privateV4(v4.slice(1).map(Number) as [number, number, number, number]);
  if (host.startsWith("[")) return privateV6(host.slice(1, -1));
  // A name with no dot is the local network's (a router, a NAS, a build host).
  return !host.includes(".");
}

function privateV4([a, b, c]: [number, number, number, number]): boolean {
  return a === 0 || a === 10 || a === 127 || a >= 224
    || (a === 100 && b >= 64 && b <= 127) || (a === 169 && b === 254) || (a === 172 && b >= 16 && b <= 31)
    || (a === 192 && b === 168) || (a === 192 && b === 0 && c === 0) || (a === 198 && (b === 18 || b === 19));
}

function privateV6(address: string): boolean {
  const groups = expandV6(address);
  if (!groups) return true;
  const [g0, g1, , , , g5, g6, g7] = groups as [number, number, number, number, number, number, number, number];
  if (groups.every((group) => group === 0) || (groups.slice(0, 7).every((group) => group === 0) && g7 === 1)) return true;
  if ((g0 & 0xffc0) === 0xfe80 || (g0 & 0xfe00) === 0xfc00 || (g0 & 0xffc0) === 0xfec0 || (g0 & 0xff00) === 0xff00) return true;
  // IPv4-mapped (::ffff:a.b.c.d) and the NAT64 prefix carry an IPv4 address in their last 32 bits.
  const embedded = groups.slice(0, 5).every((group) => group === 0) && g5 === 0xffff || (g0 === 0x64 && g1 === 0xff9b);
  return embedded && privateV4([g6 >> 8, g6 & 255, g7 >> 8, g7 & 255]);
}

/** Eight 16-bit groups of an IPv6 literal, or null when it does not parse. */
function expandV6(address: string): number[] | null {
  const [head, tail, extra] = address.split("::");
  if (extra !== undefined) return null;
  const read = (part: string) => part === "" ? [] : part.split(":").map((group) => (/^[0-9a-f]{1,4}$/.test(group) ? Number.parseInt(group, 16) : Number.NaN));
  const first = read(head ?? "");
  const last = tail === undefined ? [] : read(tail);
  const missing = 8 - first.length - last.length;
  if (tail === undefined ? missing !== 0 : missing < 1) return null;
  const groups = [...first, ...Array<number>(Math.max(0, missing)).fill(0), ...last];
  return groups.length === 8 && groups.every(Number.isInteger) ? groups : null;
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

/** A rect in the shell's CSS pixels as whole window points. */
export function toBounds(placed: BrowserRect, zoom: number): BrowserRect {
  const scale = Number.isFinite(zoom) && zoom > 0 ? zoom : 1;
  const x = Math.round(placed.x * scale);
  const y = Math.round(placed.y * scale);
  return { x, y, width: Math.max(0, Math.round((placed.x + placed.width) * scale) - x), height: Math.max(0, Math.round((placed.y + placed.height) * scale) - y) };
}
