/**
 * The web app's side of the iOS app's three plugins (`mobile/ios/App/HideShell`). Capacitor injects `window.Capacitor`
 * into the app's main frame, so nothing is bundled here and the hosted site carries no part of it; a page that is not
 * the app has no bridge and these are never called (`appHost()`). The native side answers only a call from that main
 * frame, and only for the plugins named in `BridgeGate.appPlugins`.
 */
import { appHost } from "./appHost";

interface Bridge {
  nativePromise(plugin: string, method: string, options?: Record<string, unknown>): Promise<unknown>;
  addListener(plugin: string, event: string, callback: () => void): { remove: () => void };
}

function bridge(): Bridge {
  const found = (window as unknown as { Capacitor?: Bridge }).Capacitor;
  if (!appHost() || !found || typeof found.nativePromise !== "function") throw new Error("app_bridge_missing");
  return found;
}

/** The word the native side rejects with (`keychain_failed`, `camera_denied`, `cancelled`, ...), or `unknown`. */
export function nativeCode(error: unknown): string {
  const code = (error as { code?: unknown } | null)?.code;
  return typeof code === "string" ? code : "unknown";
}

export interface KeychainItem { noiseKey: string; credential: string | null }

/** One Keychain item per computer, found by its room; there is no call by any other key. */
export const keychain = {
  async save(room: string, item: KeychainItem): Promise<void> {
    await bridge().nativePromise("HideKeychain", "save", { room, noiseKey: item.noiseKey, ...(item.credential ? { credential: item.credential } : {}) });
  },
  async read(room: string): Promise<KeychainItem | null> {
    const answer = await bridge().nativePromise("HideKeychain", "read", { room }) as { found?: boolean; noiseKey?: string; credential?: string };
    if (!answer.found || typeof answer.noiseKey !== "string") return null;
    return { noiseKey: answer.noiseKey, credential: typeof answer.credential === "string" ? answer.credential : null };
  },
  async remove(room: string): Promise<void> {
    await bridge().nativePromise("HideKeychain", "remove", { room });
  },
  async list(): Promise<string[]> {
    const answer = await bridge().nativePromise("HideKeychain", "list") as { rooms?: unknown };
    if (!Array.isArray(answer.rooms) || !answer.rooms.every((room) => typeof room === "string")) throw new Error("keychain_list_invalid");
    return answer.rooms as string[];
  },
};

/** The camera scanner: the text of one QR code, or a rejection with `cancelled`, `camera_denied` or `camera_unavailable`. The camera permission is asked here, on the person's tap. */
export async function scanQr(hint: string): Promise<string> {
  const answer = await bridge().nativePromise("HideScanner", "scan", { hint }) as { value?: unknown };
  if (typeof answer.value !== "string") throw new Error("scan_invalid");
  return answer.value;
}

export async function appInfo(): Promise<{ version: string; build: string }> {
  const answer = await bridge().nativePromise("HideApp", "info") as { version?: unknown; build?: unknown };
  return { version: typeof answer.version === "string" ? answer.version : "", build: typeof answer.build === "string" ? answer.build : "" };
}

/** The pairing link that opened the app, once; null when none waits. Native has checked its shape only. */
export async function takeLink(): Promise<string | null> {
  const answer = await bridge().nativePromise("HideApp", "takeLink") as { link?: unknown };
  return typeof answer.link === "string" ? answer.link : null;
}

/** Called when a link waits; the event carries no link, `takeLink` does. Returns the way to stop listening. */
export function onLink(callback: () => void): () => void {
  const handle = bridge().addListener("HideApp", "link", callback);
  return () => handle.remove();
}

/** The light or dark the page shows, so the status bar, the keyboard and the system's colors follow it. */
export async function setAppearance(scheme: "light" | "dark" | "system"): Promise<void> {
  await bridge().nativePromise("HideApp", "appearance", { scheme });
}
