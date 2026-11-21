// The rules of another machine's Hide screen in this window that need no
// Electron (PRD device-connect-iroh D-67, B20; security spec 8.3): what the
// shell may ask to show, which addresses its page may load, and where each
// load is fetched from, so each is tested on its own.

import type { BrowserRect } from "../../../web/src/host";

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

/** What the shell asks: a machine's screen over `rect`, or none. */
export type ShowRequest = { device: string; rect: BrowserRect } | null;

/** The shell's request, or undefined when it is not one. */
export function parseShow(value: unknown): ShowRequest | undefined {
  if (value === null) return null;
  if (typeof value !== "object") return undefined;
  const { device, rect } = value as Record<string, unknown>;
  if (typeof device !== "string" || !DEVICE.test(device)) return undefined;
  if (typeof rect !== "object" || rect === null) return undefined;
  const { x, y, width, height } = rect as Record<string, unknown>;
  const numbers = [x, y, width, height];
  if (!numbers.every((value) => typeof value === "number" && Number.isFinite(value))) return undefined;
  if ((width as number) < 0 || (height as number) < 0) return undefined;
  return { device, rect: { x: x as number, y: y as number, width: width as number, height: height as number } };
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

/**
 * Where a load of the page is fetched from: the page, its index or an asset,
 * through this computer's daemon (`/peer-screen/<device>/...`, which the
 * host answers over the connection), or null for anything else, which is
 * refused without reaching the daemon.
 */
export function fetchPath(url: string, device: string): string | null {
  if (!ofPage(url, device)) return null;
  const { pathname, search } = new URL(url);
  // The page's own query (`?probe=1`) is the page's to read and never sent on; an asset takes none.
  if (pathname === "/" || pathname === "/index.html") return `/peer-screen/${device}/`;
  if (search) return null;
  if (pathname.startsWith("/assets/") && !pathname.includes("..")) return `/peer-screen/${device}${pathname}`;
  return null;
}

/** The screen socket's address on this computer's daemon. */
export function socketAddress(origin: string, device: string): string {
  return `${origin.replace(/^http/, "ws")}/peer-screen/${device}/ws`;
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
