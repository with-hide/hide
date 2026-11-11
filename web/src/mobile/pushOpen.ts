/**
 * A tapped notification that named an agent (`HidePush.takeOpen`): the native side kept the room and the agent from the
 * decrypted notice and the page asks for them, once. A notification the phone could not open names none, so a tap on it
 * leaves the app where it was.
 */
import { appHost } from "./appHost";
import { onPushOpen, push } from "./appBridge";

export interface PushOpenTarget { room: string; tag: string }

/** Takes the waiting target now and again whenever the app says one arrived. Returns the way to stop. */
export function followPushOpens(open: (target: PushOpenTarget) => void, ready: () => void = () => undefined): () => void {
  if (!appHost()) { ready(); return () => undefined; }
  let stopped = false;
  const take = async () => {
    try {
      const target = await push.takeOpen();
      if (target && !stopped) open({ room: target.room, tag: `${target.deviceId}|${target.paneId}` });
    } catch {
      // A bridge that is not there (an older bundle) or a call the app refused: there is nothing to open.
    }
  };
  void take().finally(ready);
  let stop: () => void;
  try { stop = onPushOpen(() => void take()); } catch { return () => { stopped = true; }; }
  return () => { stopped = true; stop(); };
}

/** The start page's address for one agent of one computer: its loader finds the computer and hands the tag on. */
export function pushOpenAddress(target: PushOpenTarget): string {
  return `/?${new URLSearchParams({ host: target.room, open: target.tag })}`;
}
