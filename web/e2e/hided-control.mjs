// A fixture's or a measurement's own connection to its daemon's WebSocket: it
// sends the typed events the web shell sends and reads the snapshot frames the
// shell reads, so panes are made and checked the same way on every engine
// (PRD engine-bundle-3-core-owns D-24). It is never the page's connection.
// Shared by the e2e fixture (`engine-fixture.ts`) and the web shell
// measurement (`scripts/web-shell-measure/fixture.mjs`, `restore.mjs`); plain
// JavaScript so the measurement runs it with Node.
import fs from "node:fs";
import path from "node:path";
import { clearTimeout, setTimeout } from "node:timers";

/**
 * Opens the connection to the daemon at `origin` with its `token` and returns
 * once the daemon sent its first snapshot. `until` waits `ms` unless it names
 * its own bound.
 */
export async function connectControl({ origin, token }, ms = 15_000, terminalEvidence) {
  // The daemon accepts its own origin only; Node's WebSocket takes undici's headers.
  const ws = new globalThis.WebSocket(`${origin.replace(/^http/, "ws")}/ws`, { headers: { origin } });
  let rest = null;
  let lastTaskKey = "";
  let closed = null;
  // Each `path_refused` frame the daemon answered, oldest first.
  const refusals = [];
  const waiters = new Set();
  const settle = () => {
    if (!rest) return;
    for (const waiter of [...waiters]) if (waiter.check(rest)) waiters.delete(waiter);
  };
  const failAll = (error) => {
    for (const waiter of [...waiters]) waiter.fail(error);
    waiters.clear();
  };
  await new Promise((resolve, reject) => {
    ws.onerror = () => reject(new Error(`the connection to ${origin} failed`));
    ws.onclose = () => {
      closed = new Error(`the connection to ${origin} closed`);
      reject(closed);
      failAll(closed);
    };
    ws.onopen = () => ws.send(JSON.stringify({ token, schema_version: 2 }));
    ws.onmessage = (event) => {
      if (typeof event.data !== "string") return;
      const frame = JSON.parse(event.data);
      if (terminalEvidence && frame.payload?.chunks) terminalEvidence(frame.payload.chunks);
      if (frame.type === "path_refused") {
        refusals.push(frame.payload);
        settle();
        return;
      }
      // A snapshot carries the whole `rest`, a delta its changed top-level fields whole.
      if (frame.type === "snapshot" && frame.payload?.rest) rest = frame.payload.rest;
      else if (frame.type === "delta" && frame.payload?.rest && rest) rest = { ...rest, ...frame.payload.rest };
      else return;
      if (process.env.HIDE_E2E_SCREENSHOT_DIR) {
        const op = rest.task_operation;
        const key = op ? `${op.id}/${op.request_id}/${op.phase}/${op.agent_phase}` : "none";
        if (key !== lastTaskKey) {
          lastTaskKey = key;
          fs.appendFileSync(path.join(process.env.HIDE_E2E_SCREENSHOT_DIR, "task-trace.log"), `${Date.now()} ${origin} ${frame.type} ${key} err=${rest.status?.last_error?.kind}/${rest.status?.last_error?.request_id}\n`);
        }
      }
      resolve();
      settle();
    };
  });
  return {
    refusals,
    send: (kind, payload) => {
      if (closed) throw closed;
      if (process.env.HIDE_E2E_SCREENSHOT_DIR && kind === "agent_start_in_checkout") {
        fs.appendFileSync(path.join(process.env.HIDE_E2E_SCREENSHOT_DIR, "task-trace.log"), `${Date.now()} ${origin} SEND ${payload.request_id} seen=${rest?.task_operation ? `${rest.task_operation.id}/${rest.task_operation.request_id}/${rest.task_operation.phase}/${rest.task_operation.agent_phase}` : "none"}\n`);
      }
      ws.send(JSON.stringify({ schema_version: 2, kind, payload }));
    },
    until: (read, what, wait = ms) => new Promise((resolve, reject) => {
      if (closed) return reject(closed);
      const timer = setTimeout(() => {
        waiters.delete(waiter);
        reject(new Error(`timed out after ${wait} ms waiting for ${what}`));
      }, wait);
      const waiter = {
        check: (now) => {
          let value;
          // A read that throws ends its wait with that error.
          try {
            value = read(now);
          } catch (error) {
            clearTimeout(timer);
            reject(error);
            return true;
          }
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
      failAll(new Error("the connection was closed"));
      ws.close();
    },
  };
}

/** `connectControl` for the daemon whose state folder is `stateDir`, as its `hided.json` names it. */
export function connectStateDir(stateDir, ms) {
  const { port, token } = JSON.parse(fs.readFileSync(path.join(stateDir, "hided.json"), "utf8"));
  return connectControl({ origin: `http://127.0.0.1:${port}`, token }, ms);
}
