// The rules of another machine's Hide screen in this window that need no
// Electron (PRD device-connect-iroh D-67, B20; security spec 8.3): what the
// shell may ask to show, which addresses its page may load, and where each
// load is fetched from, so each is tested on its own.

import type { BrowserRect } from "../../../web/src/host";
import { readDisplayPreferences, type DisplayPreferences } from "../../../web/src/displayPreferences";
import type { RemotePlace, RemoteWork, RemoteWorkResult } from "../../../web/src/remoteView";

/** The page's own scheme: `hide-remote://<device>/`, one origin per machine. */
export const REMOTE_SCHEME = "hide-remote";

/** The close a hidden tile's socket gets, which the page answers by waiting to be shown (web `remoteView.ts`). */
export const HIDDEN_CLOSE = 4012;

/** The largest frame the page may send; an attachment's bytes are the largest thing it sends. */
export const MAX_SEND_BYTES = 32 * 1024 * 1024;

/**
 * What may wait unsent on a view's socket: one largest frame and 8 MiB beside
 * it. A host that stops reading would grow the queue without end in this
 * app's main process, so past it the socket is closed instead.
 */
export const MAX_QUEUED_BYTES = MAX_SEND_BYTES + 8 * 1024 * 1024;

/** Whether a frame of `size` bytes may join `queued` bytes waiting on the socket. */
export function queueAllows(queued: number, size: number): boolean {
  return queued + size <= MAX_QUEUED_BYTES;
}

/** A device id as hided names one: letters, digits and `-`, never an address. */
const DEVICE = /^[a-z0-9][a-z0-9-]{0,127}$/;

/** A machine's key as Hide spells it everywhere: 64 lower hex characters. */
const KEY = /^[0-9a-f]{64}$/;

/**
 * What the shell asks: a machine's screen over `rect`, or none. `key` is the
 * key that machine is paired under now; a page is kept for one key only, so
 * a page one host loaded never reaches a host that later took the same
 * device id (security pre-review S1).
 */
/**
 * `name`: what this window calls the machine, which its page names itself by;
 * null when the shell gave none. `display`: this window's View menu choices,
 * which its page draws with; null when the shell gave none it knows.
 */
export type ShowRequest = { device: string; key: string; rect: BrowserRect; name: string | null; display: DisplayPreferences | null } | null;

/** The longest machine name a show carries, as a pairing keeps one. */
const MAX_NAME_CHARS = 128;

/** The shell's request, or undefined when it is not one. */
export function parseShow(value: unknown): ShowRequest | undefined {
  if (value === null) return null;
  if (typeof value !== "object") return undefined;
  const { device, key, rect, name, display } = value as Record<string, unknown>;
  if (typeof device !== "string" || !DEVICE.test(device)) return undefined;
  if (typeof key !== "string" || !KEY.test(key)) return undefined;
  if (typeof rect !== "object" || rect === null) return undefined;
  const { x, y, width, height } = rect as Record<string, unknown>;
  const numbers = [x, y, width, height];
  if (!numbers.every((value) => typeof value === "number" && Number.isFinite(value))) return undefined;
  if ((width as number) < 0 || (height as number) < 0) return undefined;
  // A name that is no short line is left out; the page then names itself as its own machine does.
  const named = typeof name === "string" && name.trim() !== "" && Array.from(name).length <= MAX_NAME_CHARS && !/\p{Cc}/u.test(name) ? name : null;
  return { device, key, rect: { x: x as number, y: y as number, width: width as number, height: height as number }, name: named, display: readDisplayPreferences(display) };
}

/** The most machines the paired set names; a window holds far fewer. */
const MAX_PAIRED = 256;

/**
 * The machines this window's operator has paired with, each `pairName`d by
 * its device and the key it was paired under, or null when the shell's list
 * is out of shape: nothing is cleared on a list that could not be read.
 */
export function parsePaired(value: unknown): { core: string; pairs: Set<string> } | null {
  if (typeof value !== "object" || value === null) return null;
  const { core, pairs: rows } = value as Record<string, unknown>;
  if (typeof core !== "string" || !DEVICE.test(core) || !Array.isArray(rows) || rows.length > MAX_PAIRED) return null;
  const pairs = new Set<string>();
  for (const row of rows) {
    if (typeof row !== "object" || row === null) return null;
    const { device, key } = row as Record<string, unknown>;
    if (typeof device !== "string" || !DEVICE.test(device) || typeof key !== "string" || !KEY.test(key)) return null;
    pairs.add(`${device}\u0000${key}`);
  }
  return { core, pairs };
}

/** The longest path, pane, model or profile name an ask carries. */
const MAX_NAME = 4096;

/** The longest prompt a start carries, as the operator typed it. */
export const MAX_PROMPT = 1024 * 1024;

/**
 * The operator's ask of a machine's screen (D-73, D-78): its work to show or
 * a start to make there, for the key that machine is paired under now, so an
 * ask never reaches a host that later took the same device id (S1).
 */
export type GoRequest = { device: string; key: string; work: RemoteWork };

function named(value: unknown, max = MAX_NAME): value is string {
  return typeof value === "string" && value.length > 0 && value.length <= max;
}

/** A name that may be absent: null when absent, undefined when it is no name. */
function maybe(value: unknown, max = MAX_NAME): string | null | undefined {
  if (value === null || value === undefined) return null;
  return named(value, max) ? value : undefined;
}

function parsePlace(value: unknown): RemotePlace | undefined {
  if (typeof value !== "object" || value === null) return undefined;
  const place = value as Record<string, unknown>;
  if ("checkout" in place) return named(place.checkout) ? { checkout: place.checkout } : undefined;
  const base = maybe(place.base);
  if (!named(place.repository) || !named(place.branch) || base === undefined) return undefined;
  return { repository: place.repository, branch: place.branch, base };
}

function parseWork(value: unknown): RemoteWork | undefined {
  if (typeof value !== "object" || value === null) return undefined;
  const work = value as Record<string, unknown>;
  if (work.op === "show") {
    // A show with a request is answered by the page (D-55); one without is shown as before.
    if (!named(work.pane) || (work.request !== undefined && !named(work.request, 128))) return undefined;
    return work.request === undefined ? { op: "show", pane: work.pane } : { op: "show", pane: work.pane, request: work.request };
  }
  if (work.op !== "start") return undefined;
  const place = parsePlace(work.place);
  const model = maybe(work.model);
  const prompt = maybe(work.prompt, MAX_PROMPT);
  const profile = maybe(work.profile);
  if (place === undefined || model === undefined || prompt === undefined || profile === undefined) return undefined;
  if (!named(work.request, 128) || !named(work.provider, 64)) return undefined;
  return { op: "start", request: work.request, place, provider: work.provider, model, prompt, profile };
}

/** The shell's ask, or undefined when it is not one. */
export function parseGo(value: unknown): GoRequest | undefined {
  if (typeof value !== "object" || value === null) return undefined;
  const { device, key, work } = value as Record<string, unknown>;
  if (typeof device !== "string" || !DEVICE.test(device)) return undefined;
  if (typeof key !== "string" || !KEY.test(key)) return undefined;
  const parsed = parseWork(work);
  return parsed ? { device, key, work: parsed } : undefined;
}

/** A word a result carries (a refusal kind, a failure code or reason): short, or none. */
function word(value: unknown): string | null | undefined {
  return maybe(value, 256);
}

/** What a screen's page says of a start it was handed, or undefined when it is not that. */
export function parseWorkResult(value: unknown): RemoteWorkResult | undefined {
  if (typeof value !== "object" || value === null) return undefined;
  const result = value as Record<string, unknown>;
  if (!named(result.request, 128)) return undefined;
  const request = result.request;
  switch (result.outcome) {
    case "shown":
    case "not_listed":
    case "started":
    case "starting":
    case "unconfirmed":
    case "busy":
    case "not_shown":
    case "late":
    case "no_answer":
      return { request, outcome: result.outcome };
    case "not_added":
    case "refused":
      return named(result.kind, 256) ? { request, outcome: result.outcome, kind: result.kind } : undefined;
    case "failed": {
      const code = word(result.code);
      return code === undefined ? undefined : { request, outcome: "failed", code };
    }
    case "agent_failed": {
      const reason = word(result.reason);
      const line = maybe(result.line, MAX_NAME);
      return reason === undefined || line === undefined ? undefined : { request, outcome: "agent_failed", reason, line };
    }
    default:
      return undefined;
  }
}

/** The page's first address. */
export function pageAddress(device: string): string {
  return `${REMOTE_SCHEME}://${device}/`;
}

/** Whether `url` is an address of `device`'s page, the only place its page may go or load from. */
export function ofPage(url: string, device: string): boolean {
  try {
    const parsed = new URL(url);
    return parsed.protocol === `${REMOTE_SCHEME}:` && parsed.host === device;
  } catch {
    return false;
  }
}

/** The page's own address on this computer's daemon, which names the key the page was opened for. */
export function pagePath(device: string, key: string): string {
  return `/peer-screen/${device}/?key=${key}`;
}

/**
 * Where a load of the page is fetched from: the page, its index or an asset,
 * through this computer's daemon (`/peer-screen/<device>/...?key=<key>`,
 * which the host answers over the connection while `device` is paired under
 * `key`), or null for anything else, which is refused without reaching the
 * daemon.
 */
export function fetchPath(url: string, device: string, key: string): string | null {
  if (!ofPage(url, device)) return null;
  const { pathname, search } = new URL(url);
  // The page's own query (`?probe=1`) is the page's to read and never sent on; an asset takes none.
  if (pathname === "/" || pathname === "/index.html") return pagePath(device, key);
  if (search) return null;
  if (pathname.startsWith("/assets/") && !pathname.includes("..")) return `/peer-screen/${device}${pathname}?key=${key}`;
  return null;
}

/** The screen socket's address on this computer's daemon, for the key its page was opened for. */
export function socketAddress(origin: string, device: string, key: string): string {
  return `${origin.replace(/^http/, "ws")}/peer-screen/${device}/ws?key=${key}`;
}

/**
 * Whether a request the page's session makes may go: its own page and what
 * Chromium makes in memory. Everything else (the network, this computer's
 * loopback, a file) is refused (B20).
 */
export function requestAllowed(url: string, device: string): boolean {
  if (ofPage(url, device)) return true;
  return url.startsWith("data:") || url.startsWith("blob:") || url.startsWith("devtools:");
}
