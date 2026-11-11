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

/** The words the native scanner draws (`ScannerText`): it owns no catalog, so the page sends them in its language. */
export interface ScannerWords { title: string; hint: string; whereTitle: string; whereBody: string; deniedTitle: string; deniedBody: string; openSettings: string; close: string }

/** The camera scanner: the text of one Hide pairing link, or a rejection with `cancelled`, `busy` or `invalid_text`. The camera permission is asked here, on the person's tap, and a camera that is refused or unusable is the scanner's own guidance screen, closed by the person. */
export async function scanQr(text: ScannerWords): Promise<string> {
  const answer = await bridge().nativePromise("HideScanner", "scan", { text: { ...text } }) as { value?: unknown };
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

/** What the computer needs to encrypt a notice for this phone; the private half never leaves the Keychain (`HidePush`). */
export interface PushRegistration { token: string; environment: "sandbox" | "production"; p256dh: string; auth: string }
export type PushPermission = "unknown" | "granted" | "denied";

function registrationOf(answer: { found?: boolean; token?: unknown; environment?: unknown; p256dh?: unknown; auth?: unknown }): PushRegistration | null {
  if (!answer.found) return null;
  const { token, environment, p256dh, auth } = answer;
  if (typeof token !== "string" || typeof p256dh !== "string" || typeof auth !== "string" || (environment !== "sandbox" && environment !== "production")) throw new Error("push_registration_invalid");
  return { token, environment, p256dh, auth };
}

/** The app's notifications: allowed or not, one computer's key and registration, and the agent a tap named. There is no call that reads a private key or sends an action. */
export const push = {
  async state(): Promise<PushPermission> {
    const answer = await bridge().nativePromise("HidePush", "state") as { permission?: unknown };
    return answer.permission === "granted" || answer.permission === "denied" ? answer.permission : "unknown";
  },
  /** Asks the system for permission (only now, from the person's tap), registers with Apple and makes the computer's key. Rejects `denied`, `token_timeout`, `token_unavailable` or `keychain_failed`. */
  async enable(room: string): Promise<PushRegistration> {
    const registration = registrationOf(await bridge().nativePromise("HidePush", "enable", { room }) as never);
    if (!registration) throw new Error("push_registration_missing");
    return registration;
  },
  /** The registration to send again, or null when the system no longer allows notifications or this computer has no key. */
  async registration(room: string): Promise<PushRegistration | null> {
    return registrationOf(await bridge().nativePromise("HidePush", "registration", { room }) as never);
  },
  /** Deletes a computer's key: its notices open no more. */
  async forget(room: string): Promise<void> {
    await bridge().nativePromise("HidePush", "forget", { room });
  },
  /** The agent a tapped notification named, once. */
  async takeOpen(): Promise<{ room: string; deviceId: string; paneId: string } | null> {
    const answer = await bridge().nativePromise("HidePush", "takeOpen") as { room?: unknown; device_id?: unknown; pane_id?: unknown };
    return typeof answer.room === "string" && typeof answer.device_id === "string" && typeof answer.pane_id === "string"
      ? { room: answer.room, deviceId: answer.device_id, paneId: answer.pane_id } : null;
  },
  /** The text of a reply to this agent that did not arrive (kept an hour), once, to put back in the box it was typed in. */
  async takeReply(room: string, key: { device_id: string; pane_id: string }): Promise<string | null> {
    const answer = await bridge().nativePromise("HidePush", "takeReply", { room, device_id: key.device_id, pane_id: key.pane_id }) as { found?: boolean; text?: unknown };
    return answer.found && typeof answer.text === "string" ? answer.text : null;
  },
  /** The demo's sample notification: a local one, with no token, key or network, worded by the app from a real notice's fields. */
  async demoSample(words: { title: string; status_code: string; state: string; detail: string; place: string }): Promise<void> {
    await bridge().nativePromise("HidePush", "demoSample", { words });
  },
  /** iOS Settings for this app, where a refused permission is turned on again. */
  async openSettings(): Promise<void> {
    await bridge().nativePromise("HidePush", "openSettings");
  },
};

/** Called when a tap named an agent; the event carries no target, `takeOpen` does. Returns the way to stop listening. */
export function onPushOpen(callback: () => void): () => void {
  const handle = bridge().addListener("HidePush", "open", callback);
  return () => handle.remove();
}
