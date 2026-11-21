/**
 * A Hide pairing link that opened the iOS app (a QR scanned with the camera app, a link in a message). The native side
 * kept it after checking only its shape; here it goes through the decoder every pairing uses before anything else
 * happens, and what comes of it is the pairing screen, never a connection: a person presses Connect.
 */
import { decodeQr, type PairingQr } from "../../../hide-relay-protocol/web";
import { appHost } from "./appHost";
import { onLink, takeLink } from "./appBridge";
import { relayReady } from "./relayRuntime";

/**
 * Takes the waiting link now and again whenever the app says one arrived. `accept` gets the decoded pairing and the
 * link it came from; `refuse` is called for a link the decoder will not read. Returns the way to stop.
 */
export function followNativeLinks(accept: (pair: PairingQr, link: string) => void, refuse: () => void): () => void {
  if (!appHost()) return () => undefined;
  let stopped = false;
  const take = async () => {
    try {
      const link = await takeLink();
      if (!link || stopped) return;
      await relayReady();
      const pair = decodeQr(link);
      if (!stopped) accept(pair, link);
    } catch (error) {
      // A bridge that is not there (an older bundle) or a link the decoder refuses: the person sees only the second.
      if (error instanceof Error && error.message === "app_bridge_missing") return;
      if (!stopped) refuse();
    }
  };
  void take();
  let stop: () => void;
  try { stop = onLink(() => void take()); } catch { return () => { stopped = true; }; }
  return () => { stopped = true; stop(); };
}
