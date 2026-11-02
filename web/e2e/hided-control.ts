// A fixture's own connection to its daemon's WebSocket: it sends the typed
// events the web shell sends and reads the snapshot frames the shell reads,
// so a fixture makes and checks its panes the same way on every engine
// (PRD engine-bundle-3-core-owns D-24). It is never the page's connection.

import type { Daemon } from "./hided-fixture";

/** The snapshot's `rest`, as the frames so far have built it. */
export type Rest = Record<string, unknown>;

export type HidedControl = {
  /** Sends one client event, as `web/src/actions.ts` dispatches it. */
  send: (kind: string, payload: Record<string, unknown>) => void;
  /**
   * Resolves with `read(rest)` once it answers anything but undefined, null
   * or false for the snapshot as the frames have built it, checking each
   * frame as it lands; rejects naming `what` after `ms`.
   */
  until: <T>(read: (rest: Rest) => T | undefined | null | false, what: string, ms?: number) => Promise<T>;
  close: () => void;
};

type Waiter = { check: (rest: Rest) => boolean; fail: (error: Error) => void };

/** Opens the connection and returns once the daemon sent its first snapshot. */
export async function connectControl(daemon: Daemon): Promise<HidedControl> {
  // Node's WebSocket takes undici's options: the daemon accepts its own origin only.
  const Socket = WebSocket as unknown as new (url: string, options: { headers: Record<string, string> }) => WebSocket;
  const ws = new Socket(`${daemon.origin.replace(/^http/, "ws")}/ws`, { headers: { origin: daemon.origin } });
  let rest: Rest | null = null;
  let closed: Error | null = null;
  const waiters = new Set<Waiter>();
  const settle = () => {
    if (!rest) return;
    for (const waiter of [...waiters]) if (waiter.check(rest)) waiters.delete(waiter);
  };
  const first = new Promise<void>((resolve, reject) => {
    ws.onerror = () => reject(new Error(`the fixture's connection to ${daemon.origin} failed`));
    ws.onclose = () => {
      closed = new Error(`the fixture's connection to ${daemon.origin} closed`);
      reject(closed);
      for (const waiter of [...waiters]) waiter.fail(closed);
      waiters.clear();
    };
    ws.onopen = () => ws.send(JSON.stringify({ token: daemon.token, schema_version: 2 }));
    ws.onmessage = (event) => {
      if (typeof event.data !== "string") return;
      const frame = JSON.parse(event.data) as { type?: string; payload?: { rest?: Rest } };
      // A snapshot carries the whole `rest`, a delta its changed top-level fields whole.
      if (frame.type === "snapshot" && frame.payload?.rest) rest = frame.payload.rest;
      else if (frame.type === "delta" && frame.payload?.rest && rest) rest = { ...rest, ...frame.payload.rest };
      else return;
      resolve();
      settle();
    };
  });
  await first;
  return {
    send: (kind, payload) => {
      if (closed) throw closed;
      ws.send(JSON.stringify({ schema_version: 2, kind, payload }));
    },
    until: <T>(read: (rest: Rest) => T | undefined | null | false, what: string, ms = 15_000) =>
      new Promise<T>((resolve, reject) => {
        if (closed) return reject(closed);
        const timer = setTimeout(() => {
          waiters.delete(waiter);
          reject(new Error(`timed out after ${ms} ms waiting for ${what}`));
        }, ms);
        const waiter: Waiter = {
          check: (now) => {
            const value = read(now);
            if (value === undefined || value === null || value === false) return false;
            clearTimeout(timer);
            resolve(value);
            return true;
          },
          fail: (error) => {
            clearTimeout(timer);
            reject(error);
          },
        };
        waiters.add(waiter);
        settle();
      }),
    close: () => {
      ws.onclose = null;
      for (const waiter of [...waiters]) waiter.fail(new Error("the fixture's connection was closed"));
      waiters.clear();
      ws.close();
    },
  };
}
