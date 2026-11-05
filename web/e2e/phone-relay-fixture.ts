// A real Noise responder behind a local byte-forwarding relay. The device's
// application is a fixture; every browser crypto byte goes through the shared Rust crate.
import { spawn } from "node:child_process";
import { createECDH } from "node:crypto";
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { WebSocketServer, WebSocket } from "ws";
import { fixtureExecutable } from "./platform-fixture.ts";
import { ownUntilWorkerExit } from "./worker-owned.ts";
import { PHONE_PROTOCOL } from "../src/mobile/phoneVersion";
import type { PhoneAgent, ServerFrame } from "../src/mobile/protocol";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
type Reply = { qr?: string; frames: string[]; text: string | null };

async function devicePeer() {
  const child = spawn(path.join(repo, "target/debug/examples", fixtureExecutable("phone_peer")), [], { stdio: ["pipe", "pipe", "pipe"], windowsHide: true });
  const owner = ownUntilWorkerExit(() => { child.stdin.destroy(); child.kill(); });
  const exited = new Promise<void>((resolve) => { child.once("close", () => resolve()); });
  let buffer = "";
  let error: Error | null = null;
  let waiting: { resolve: (reply: Reply) => void; reject: (error: Error) => void; timer: ReturnType<typeof setTimeout> } | null = null;
  const replies: Reply[] = [];
  const fail = (reason: Error) => { error = reason; if (waiting) { clearTimeout(waiting.timer); waiting.reject(reason); waiting = null; } owner.stop(); };
  child.on("error", fail);
  child.on("close", () => { if (waiting) fail(new Error("phone peer ended before replying")); });
  child.stderr.on("data", () => fail(new Error("phone peer reported an error")));
  child.stdout.on("data", (chunk: Buffer) => {
    buffer += chunk.toString();
    if (buffer.length > 800_000) return fail(new Error("phone peer output limit"));
    while (buffer.includes("\n")) {
      const end = buffer.indexOf("\n");
      const line = buffer.slice(0, end); buffer = buffer.slice(end + 1);
      try {
        const reply = JSON.parse(line) as Reply;
        if (waiting) { const done = waiting; waiting = null; clearTimeout(done.timer); done.resolve(reply); }
        else if (replies.length < 1) replies.push(reply);
        else fail(new Error("unexpected phone peer output"));
      } catch { fail(new Error("invalid phone peer output")); }
    }
  });
  const next = () => new Promise<Reply>((resolve, reject) => {
    if (error) return reject(error);
    if (replies.length) return resolve(replies.shift()!);
    if (waiting) return reject(new Error("concurrent peer read"));
    const timer = setTimeout(() => fail(new Error("phone peer deadline")), 10_000);
    waiting = { resolve, reject, timer };
  });
  let tail: Promise<unknown> = Promise.resolve();
  let queued = 0;
  const call = (command: object): Promise<Reply> => {
    if (++queued > 128) { --queued; return Promise.reject(new Error("phone peer queue limit")); }
    const work = tail.then(() => {
      const response = next();
      child.stdin.write(JSON.stringify(command) + "\n");
      return response;
    });
    tail = work.catch(() => undefined);
    return work.finally(() => { --queued; });
  };
  try {
    const first = await next();
    if (!first.qr) throw new Error("missing QR");
    return { qr: first.qr, call, stop: async () => { owner.stop(); await exited; } };
  } catch (cause) { owner.stop(); await exited; throw cause; }
}

export const agent: PhoneAgent = {
  device_id: "test-device", pane_id: "pane-one", root_pane_id: "pane-one", group: "needs_you", mark: "turn",
  wait: null, descendants_working: 0, agent_kind: "claude", title: "Review the mobile connection", place: "sample-project", device_label: null,
  changed_at_unix_ms: null, line: { text: "Ready for your reply", mode: "request", tone: "subtle" }, status_code: "question",
  demand: "question", emphasized: true, holds_notification: true,
};

export async function startPhoneRelay(port = 0) {
  const peer = await devicePeer();
  const pairing = JSON.parse(Buffer.from(peer.qr.split("#pair=")[1]!, "base64url").toString()) as { room: string; name: string; code: string; noise_key: string };
  const credential = "test-phone-credential";
  const vapid = createECDH("prime256v1"); vapid.generateKeys();
  const received: Record<string, unknown>[] = [];
  const appliedInputs: string[] = [];
  const plaintext: string[] = [];
  const errors: string[] = [];
  const active = new Map<number, WebSocket>();
  let nextConnection = 1;
  const state = { protocol: PHONE_PROTOCOL, offline: false, refusal: "", sleeping: false, push: false, dropInputReply: false, refuseMission: "" };
  const wire = { binaryFrames: 0 };
  const groups = (): ServerFrame => ({ type: "agents", interface_language: null, color_theme: null, devices: [{ device_id: "local", label: null, local: true, connected: true, needs_you: 1, working: 2 }], groups: [
    { group: "needs_you", count: 1, rows: [{ kind: "agent", ...agent, ...(state.sleeping ? { sleep: { state: "sleeping", since_unix_ms: 1 } } : {}) } as PhoneAgent & { kind: "agent" }] },
    { group: "working", count: 2, rows: [] },
  ] });
  // One mission that waits on the Conductor's question, as the core's `missions` frame lists it.
  const missions = (): ServerFrame => ({ type: "missions", failed: false, items: [{
    id: "mission-one", title: "Ship the release", device_id: "test-device", device_label: null, status: "running", decision: "Sign the build?",
    question: { key: 2, why: "The certificate expired", options: ["Sign it", "Wait"], recommend: 1, node: null, asked_at_unix_ms: 1 },
    answer: null, answer_failed: null, landing_order: [], created_at_unix_ms: 1, started_at_unix_ms: 1, closed_at_unix_ms: null, stale: false,
    conductor: { present: true, takes_answers: true, name: "Conductor", agent_kind: "claude", agent: null },
    nodes: [{ key: "a", title: "Bump the version", kind: "change", depends_on: [], status: "done", blocked: null, delivery: "pr", pr: 7, pr_state: "merged", base_pr: null, stage: null, attention: null, landing: null, agent_runtime: "ended", worker_agent: "claude", agent: null, escalation: null }],
  }] });
  const server = createServer((request, response) => {
    void (async () => {
      const pathname = new URL(request.url!, "http://localhost").pathname;
      const file = path.resolve(repo, "web/phone-dist", "." + (pathname.endsWith("/") ? pathname + "index.html" : pathname));
      if (!file.startsWith(path.join(repo, "web/phone-dist") + path.sep)) { response.writeHead(403).end(); return; }
      const types: Record<string, string> = { ".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm", ".json": "application/json", ".webmanifest": "application/manifest+json", ".css": "text/css", ".png": "image/png", ".woff2": "font/woff2" };
      response.setHeader("Content-Type", types[path.extname(file)] ?? "application/octet-stream");
      response.setHeader("Cache-Control", "no-store");
      response.end(await readFile(file));
    })().catch(() => { response.writeHead(404).end(); });
  });
  const relay = new WebSocketServer({ server, maxPayload: 65_536 });
  const send = async (id: number, frame: ServerFrame) => {
    const response = await peer.call({ op: "send", connection: id, text: JSON.stringify(frame) });
    const ws = active.get(id);
    if (ws?.readyState === WebSocket.OPEN) for (const bytes of response.frames) ws.send(Buffer.from(bytes, "base64url"));
  };
  const handle = async (id: number, data: Record<string, unknown>) => {
    if (received.length >= 1000) throw new Error("fixture message limit");
    received.push(data);
    if (data.client_kind === "phone") {
      if (state.refusal || (data.pair !== pairing.code && data.token !== credential)) {
        await send(id, { type: "refused", reason: state.refusal || "revoked" }); active.get(id)?.close(); return;
      }
      if (data.pair) await send(id, { type: "paired", credential, phone_id: "phone-one", name: "Phone" });
      await send(id, { type: "hello", mac_name: pairing.name, phone_id: "phone-one", name: "Phone", vapid_public_key: vapid.getPublicKey().toString("base64url"), notifications: "unasked" });
      await send(id, { type: "meta", push_mode: state.push ? "app_closed" : "off", other_phones: 0 });
      await send(id, groups());
      await send(id, missions());
    } else if (data.type === "ping") await send(id, { type: "pong", id: Number(data.id) });
    else if (data.type === "open" || data.type === "view") {
      await send(id, { type: "conversation", device_id: agent.device_id, pane_id: agent.pane_id, state: "ok", mode: "reset", messages: [{ id: 1, who: "agent", text: "The encrypted connection is ready.", truncated: false, at_ms: 1 }], before: null });
      await send(id, { type: "rows", device_id: agent.device_id, pane_id: agent.pane_id, state: "ok", text: "terminal output from device", lines: 1, more: false });
    } else if (data.type === "push_subscription") await send(id, { type: "push_state", notifications: "on" });
    else if (data.type === "input") {
      appliedInputs.push(String(data.text ?? data.key));
      // The device applied the input, but the acknowledgement never reaches the phone.
      if (state.dropInputReply) { state.offline = true; active.get(id)?.close(); return; }
      await send(id, { type: "input_result", request_id: String(data.request_id), ok: true, reason: null });
    }
    else if (data.type === "start_sheet" && data.open) await send(id, { type: "start_catalog", targets: [{ id: "home:test-device", device_id: "test-device", place: null, device_label: null, connected: true }], core_platform: "macos aarch64", kinds: [{ id: "claude", models: [] }], remembered: { kind: "claude", models: {} } });
    else if (data.type === "start_agent") await send(id, { type: "start_result", request_id: String(data.request_id), ok: true, reason: null, device_id: agent.device_id, pane_id: agent.pane_id });
    else if (data.type === "answer_mission") await send(id, { type: "mission_result", request_id: String(data.request_id), mission: String(data.mission), ok: !state.refuseMission, reason: state.refuseMission || null });
    else if (data.type === "wake_agent") { state.sleeping = false; await send(id, { type: "sleep_result", request_id: String(data.request_id), sleep_id: `${agent.device_id}|${agent.pane_id}`, ok: true, reason: null }); await send(id, groups()); }
  };
  relay.on("connection", (ws) => {
    let id: number | null = null;
    let queue: Promise<unknown> = Promise.resolve();
    let pending = 0;
    const enqueue = (work: () => Promise<void>) => {
      if (++pending > 64) { ws.close(); return; }
      queue = queue.then(work).catch((cause) => { errors.push(String(cause)); ws.close(); }).finally(() => { --pending; });
    };
    ws.on("message", (bytes, binary) => enqueue(async () => {
      if (!binary) {
        const text = bytes.toString(); plaintext.push(text);
        const frame = JSON.parse(text);
        if (id !== null || frame.type !== "phone" || frame.room !== pairing.room || state.offline) { ws.close(); return; }
        id = nextConnection++;
        await peer.call({ op: "open", connection: id });
        active.set(id, ws);
        ws.send(JSON.stringify({ type: "joined", connection: id, phone_protocol: state.protocol }));
      } else {
        wire.binaryFrames++;
        if (id === null) throw new Error("missing join");
        const reply = await peer.call({ op: "receive", connection: id, frame: Buffer.from(bytes as Buffer).toString("base64url") });
        if (ws.readyState === WebSocket.OPEN) for (const frame of reply.frames) ws.send(Buffer.from(frame, "base64url"));
        if (reply.text) await handle(id, JSON.parse(reply.text));
      }
    }));
    ws.on("close", () => enqueue(async () => { if (id !== null) { active.delete(id); await peer.call({ op: "close", connection: id }); } }));
    ws.on("error", () => ws.close());
  });
  try {
    await new Promise<void>((resolve, reject) => { server.once("error", reject); server.listen(port, "127.0.0.1", resolve); });
    const actualPort = (server.address() as { port: number }).port;
    return { relayEndpoints: new Map([[pairing.room, `ws://127.0.0.1:${actualPort}/ws`]]), origin: `http://127.0.0.1:${actualPort}`, wsUrl: `ws://127.0.0.1:${actualPort}/ws`, qr: peer.qr, pairing, credential, received, appliedInputs, plaintext, errors, state, wire,
      disconnect: () => { for (const socket of active.values()) socket.close(); },
      stop: async () => { for (const socket of relay.clients) socket.terminate(); relay.close(); server.closeAllConnections(); await new Promise<void>((resolve) => server.close(() => resolve())); await peer.stop(); },
    };
  } catch (error) { relay.close(); server.close(); await peer.stop(); throw error; }
}
