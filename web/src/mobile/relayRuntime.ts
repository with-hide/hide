import init, { decodeQr, type PairingQr } from "../../../hide-relay-protocol/web";
import wasmUrl from "../../../hide-relay-protocol/pkg/relay_bg.wasm?url";

let ready: Promise<unknown> | undefined;
export function relayReady(): Promise<unknown> {
  return ready ??= init({ module_or_path: wasmUrl });
}

/** The fragment is removed before loading WASM or doing any network work. */
export async function takePairing(): Promise<PairingQr | null> {
  const hash = window.location.hash;
  if (!hash.startsWith("#pair=")) return null;
  window.history.replaceState(null, "", window.location.pathname + window.location.search);
  await relayReady();
  return decodeQr(`https://app.withhide.dev/${hash}`);
}

export function keyBytes(key: string): Uint8Array {
  return Uint8Array.from(atob(key.replace(/-/g, "+").replace(/_/g, "/")), (c) => c.charCodeAt(0));
}

export const RELAY_URL = import.meta.env.VITE_HIDE_RELAY_URL || "wss://relay.withhide.dev/ws";
export { PHONE_PROTOCOL } from "./phoneVersion";
export const hostedPhone = () => /^\/p\/[1-9][0-9]*\//.test(window.location.pathname);
