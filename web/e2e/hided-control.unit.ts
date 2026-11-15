// What closing a fixture's daemon connection promises: it returns only once
// the daemon has closed the connection, which hided does only after applying
// every event sent on it. Before, it returned at once, and an event sent on
// a connection that a step then closed could reach the core after an event
// the next step sent on its own connection: the projects-sidebar journey's
// last tab focus landed after the next step's and took the screen back
// (with-hide/hide#2575).
//
// The daemon here is a WebSocket server that speaks the first frame a
// fixture waits for and records what it reads, behind a relay the test holds:
// while held, nothing the client sends reaches it, so whether `close` waits
// for the daemon is the test's order to decide, not the scheduler's.
import { afterEach, expect, test, vi } from "vitest";
import net from "node:net";
import { WebSocketServer } from "ws";
import { connectControl, useControl } from "./hided-control.mjs";

type Daemon = { origin: string; read: string[]; hold: () => void; release: () => void; held: () => number; stop: () => Promise<void> };

const daemons: Daemon[] = [];
afterEach(async () => {
  await Promise.all(daemons.splice(0).map((daemon) => daemon.stop()));
  vi.restoreAllMocks();
});

/** A daemon that answers the first frame with an empty snapshot and records the kind of each event it reads after it. */
async function startDaemon(): Promise<Daemon> {
  const read: string[] = [];
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 });
  server.on("connection", (socket) => {
    let first = true;
    socket.on("message", (data) => {
      if (first) {
        first = false;
        socket.send(JSON.stringify({ type: "snapshot", payload: { rest: {} } }));
        return;
      }
      read.push((JSON.parse(String(data)) as { kind: string }).kind);
    });
  });
  await new Promise<void>((resolve) => server.once("listening", () => resolve()));
  const daemonPort = (server.address() as net.AddressInfo).port;

  let holding = false;
  const heldChunks: Buffer[] = [];
  const pairs = new Set<{ client: net.Socket; upstream: net.Socket }>();
  let forward: (() => void) | null = null;
  const relay = net.createServer((client) => {
    const upstream = net.connect(daemonPort, "127.0.0.1");
    const pair = { client, upstream };
    pairs.add(pair);
    forward = () => {
      for (const chunk of heldChunks.splice(0)) upstream.write(chunk);
    };
    client.on("data", (chunk) => (holding ? heldChunks.push(chunk) : upstream.write(chunk)));
    upstream.on("data", (chunk) => client.write(chunk));
    client.on("close", () => upstream.destroy());
    upstream.on("close", () => client.destroy());
    client.on("error", () => upstream.destroy());
    upstream.on("error", () => client.destroy());
  });
  await new Promise<void>((resolve) => relay.listen(0, "127.0.0.1", () => resolve()));
  const relayPort = (relay.address() as net.AddressInfo).port;

  const daemon: Daemon = {
    origin: `http://127.0.0.1:${relayPort}`,
    read,
    hold: () => {
      holding = true;
    },
    release: () => {
      holding = false;
      forward?.();
    },
    held: () => heldChunks.length,
    stop: async () => {
      for (const { client, upstream } of pairs) {
        client.destroy();
        upstream.destroy();
      }
      await Promise.all([new Promise((resolve) => relay.close(resolve)), new Promise((resolve) => server.close(resolve))]);
    },
  };
  daemons.push(daemon);
  return daemon;
}

/** Resolves once the relay holds something the client sent. */
async function heldSomething(daemon: Daemon): Promise<void> {
  await vi.waitFor(() => expect(daemon.held()).toBeGreaterThan(0), { timeout: 5_000, interval: 5 }); // wait-ok: the relay holds a frame the client sent
}

test("closing waits until the daemon has read every event sent before it and closed the connection", async () => {
  const daemon = await startDaemon();
  const control = await connectControl({ origin: daemon.origin, token: "token" });
  daemon.hold();
  control.send("focus_tab", { workspace_id: "w", checkout_id: "c", tab_id: "h3:t8" });
  let closed = false;
  const closing = Promise.resolve(control.close()).then(() => {
    closed = true;
  });
  await heldSomething(daemon);
  // Every reaction to a close that did not wait has run by the next turn.
  await new Promise((resolve) => setImmediate(resolve));
  expect(closed, "the close returned while the daemon had read nothing of what was sent").toBe(false);
  expect(daemon.read).toEqual([]);

  daemon.release();
  await closing;
  expect(daemon.read, "the daemon had read the event when the close returned").toEqual(["focus_tab"]);
});

test("closing fails, naming the connection and what was sent on it, when the daemon never closes it", async () => {
  const daemon = await startDaemon();
  const control = await connectControl({ origin: daemon.origin, token: "token" }, 300);
  daemon.hold();
  control.send("focus_tab", { workspace_id: "w", checkout_id: "c", tab_id: "h3:t8" });
  await expect(control.close()).rejects.toThrow(new RegExp(`^the daemon did not close connection \\d+ to ${daemon.origin} within 300 ms; the event sent on it may not have been applied: focus_tab$`));
  expect(daemon.read).toEqual([]);
});

test("a failed close names the latest sixteen events sent and counts the ones before them", async () => {
  const daemon = await startDaemon();
  const control = await connectControl({ origin: daemon.origin, token: "token" }, 300);
  daemon.hold();
  for (let index = 0; index < 20; index += 1) control.send("key", { pane_id: "h1:p1", text: "x" });
  control.send("focus_tab", { workspace_id: "w", checkout_id: "c", tab_id: "h3:t8" });
  await expect(control.close()).rejects.toThrow(`the 21 events sent on it may not have been applied: 5 earlier, then ${[...Array(15).fill("key"), "focus_tab"].join(", ")}`);
});

test("a step's own failure stays the error when the close after it fails too", async () => {
  const daemon = await startDaemon();
  const control = await connectControl({ origin: daemon.origin, token: "token" }, 300);
  const reported = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
  daemon.hold();
  await expect(useControl(control, async () => {
    throw new Error("the step failed");
  })).rejects.toThrow("the step failed");
  expect(reported).toHaveBeenCalledWith(expect.stringMatching(/did not close connection \d+ to .* within 300 ms; no events were sent on it/));
});
