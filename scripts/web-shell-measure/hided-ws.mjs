// A measurement's own connection to the measured daemon's WebSocket: it sends
// the typed events the web shell sends and reads the snapshot frames the shell
// reads (`fixture.mjs`, `restore.mjs`). It is never the measured page's.
import fs from "node:fs";
import path from "node:path";

/** How long a step waits for the snapshot to answer, unless it names its own bound. */
const STEP_MS = 30_000;

/**
 * One connection to the daemon whose state folder is `stateDir`: `send` an
 * event, `until` the snapshot answers; `refusals` lists the daemon's
 * `path_refused` answers, which also wake a waiting `until`.
 */
export async function connect(stateDir, ms = STEP_MS) {
  const { port, token } = JSON.parse(fs.readFileSync(path.join(stateDir, "hided.json"), "utf8"));
  const origin = `http://127.0.0.1:${port}`;
  const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`, { headers: { origin } });
  let rest = null;
  // Each `path_refused` frame the daemon answered, oldest first.
  const refusals = [];
  const waiters = new Set();
  const settle = () => { for (const waiter of [...waiters]) if (waiter.check()) waiters.delete(waiter); };
  await new Promise((resolve, reject) => {
    ws.onerror = () => reject(new Error(`no connection to ${origin}`));
    ws.onclose = () => { for (const waiter of [...waiters]) waiter.fail(new Error("the daemon closed the connection")); waiters.clear(); };
    ws.onopen = () => ws.send(JSON.stringify({ token, schema_version: 2 }));
    ws.onmessage = (event) => {
      if (typeof event.data !== "string") return;
      const frame = JSON.parse(event.data);
      if (frame.type === "path_refused") {
        refusals.push(frame.payload);
        settle();
        return;
      }
      if (frame.type === "snapshot" && frame.payload?.rest) rest = frame.payload.rest;
      else if (frame.type === "delta" && frame.payload?.rest && rest) rest = { ...rest, ...frame.payload.rest };
      else return;
      resolve();
      settle();
    };
  });
  return {
    refusals,
    send: (kind, payload) => ws.send(JSON.stringify({ schema_version: 2, kind, payload })),
    until: (read, what, wait = ms) => new Promise((resolve, reject) => {
      const timer = setTimeout(() => { waiters.delete(waiter); reject(new Error(`timed out after ${wait} ms waiting for ${what}`)); }, wait);
      const waiter = {
        check: () => {
          let value;
          // A read that throws ends its wait with that error.
          try { value = read(rest); } catch (error) { clearTimeout(timer); reject(error); return true; }
          if (value === undefined || value === null || value === false) return false;
          clearTimeout(timer);
          resolve(value);
          return true;
        },
        fail: (error) => { clearTimeout(timer); reject(error); },
      };
      waiters.add(waiter);
      settle();
    }),
    close: () => { ws.onclose = null; ws.close(); },
  };
}

