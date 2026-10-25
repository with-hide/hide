// @platform: The real pinned CLI starts two native pane processes together;
// a partial start failure must release the private server and Windows locks.
import { expect, test } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import net from "node:net";
import path from "node:path";
import { herdrBinary, setFixtureLifecycle, startHerdr } from "./herdr-fixture";
import { herdrGate } from "./herdr-gate";
import { fixtureProgram, localEndpoint } from "./platform-fixture";

/** A CLI-side rendezvous orders the starts, without substituting any Herdr answer. */
async function withStartBarrier(failFirst: boolean, journey: (root: string) => Promise<void>): Promise<void> {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "hide-start-barrier-"));
  const binary = herdrBinary();
  const previous = process.env.HIDE_E2E_HERDR_BIN;
  try {
    const proxy = fixtureProgram(root, "herdr-start-barrier", `
const fs = require("node:fs"), path = require("node:path");
const { execFileSync } = require("node:child_process");
const root = ${JSON.stringify(root)}, binary = ${JSON.stringify(binary)};
const args = process.argv.slice(2);
if (args[0] === "server" && args.length === 1) {
  fs.writeFileSync(path.join(root, "fixture-root"), process.env.HIDE_E2E_ROOT);
  fs.writeFileSync(path.join(root, "fixture-socket"), process.env.HERDR_SOCKET_PATH);
}
if (args[0] === "agent" && args[1] === "start") {
  fs.writeFileSync(path.join(root, args[2] + ".arrived"), "");
  const deadline = Date.now() + 10000;
  while (!["one", "two"].every(name => fs.existsSync(path.join(root, name + ".arrived")))) {
    if (Date.now() >= deadline) throw new Error("second start never arrived while the first was pending");
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 25);
  }
  if (${failFirst} && args[2] === "one") args[args.indexOf("--kind") + 1] = "invalid-fixture-kind";
}
try {
  const out = execFileSync(binary, args, { env: process.env, timeout: 30000 });
  if (args[0] === "agent" && args[1] === "start") fs.writeFileSync(path.join(root, args[2] + ".completed"), "");
  process.stdout.write(out);
} catch (error) {
  if (error.stdout) process.stdout.write(error.stdout);
  if (error.stderr) process.stderr.write(error.stderr);
  process.exit(typeof error.status === "number" ? error.status : 1);
}
`);
    process.env.HIDE_E2E_HERDR_BIN = proxy;
    await journey(root);
  } finally {
    if (previous === undefined) delete process.env.HIDE_E2E_HERDR_BIN;
    else process.env.HIDE_E2E_HERDR_BIN = previous;
    fs.rmSync(root, { recursive: true, force: true });
  }
}

/** Hold the actual native shim launch, without manufacturing a Herdr state. */
async function withHeldLaunch(journey: (release: () => void) => Promise<void>): Promise<void> {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "hide-launch-gate-"));
  const binary = herdrBinary();
  const previous = process.env.HIDE_E2E_HERDR_BIN;
  const release = () => fs.writeFileSync(path.join(root, "release"), "");
  try {
    const launcher = fixtureProgram(root, "claude", `
const fs = require("node:fs"), path = require("node:path"), cp = require("node:child_process");
const deadline = Date.now() + 30000;
while (!fs.existsSync(${JSON.stringify(path.join(root, "release"))})) {
  if (Date.now() >= deadline) throw new Error("native shim launch was not released");
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 25);
}
const child = cp.spawn(path.join(process.env.HIDE_E2E_ROOT, "native", path.basename(process.argv[1]).replace(/\\.js$/, ".exe")), process.argv.slice(2), { stdio: "inherit", env: process.env });
const stopChild = () => { if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL"); };
process.once("exit", stopChild);
for (const signal of ["SIGTERM", "SIGINT"]) process.once(signal, () => { stopChild(); process.exit(1); });
child.on("error", error => { console.error(error); process.exit(1); });
child.on("exit", code => process.exit(code === null ? 1 : code));
`);
    const proxy = fixtureProgram(root, "herdr-launch-gate", `
const fs = require("node:fs"), path = require("node:path"), cp = require("node:child_process");
const args = process.argv.slice(2);
if (args[0] === "server" && args.length === 1) {
  const root = process.env.HIDE_E2E_ROOT, name = ${JSON.stringify(path.basename(launcher))};
  fs.mkdirSync(path.join(root, "native"));
  fs.renameSync(path.join(root, "bin", name), path.join(root, "native", name));
  fs.copyFileSync(${JSON.stringify(launcher)}, path.join(root, "bin", name));
  if (process.platform === "win32") fs.copyFileSync(${JSON.stringify(path.join(root, "claude.js"))}, path.join(root, "bin", "claude.js"));
}
try { process.stdout.write(cp.execFileSync(${JSON.stringify(binary)}, args, { env: process.env, stdio: ["ignore", "pipe", "pipe"] })); }
catch (error) { if (error.stderr) process.stderr.write(error.stderr); process.exit(typeof error.status === "number" ? error.status : 1); }
`);
    process.env.HIDE_E2E_HERDR_BIN = proxy;
    await journey(release);
  } finally {
    if (previous === undefined) delete process.env.HIDE_E2E_HERDR_BIN;
    else process.env.HIDE_E2E_HERDR_BIN = previous;
    fs.rmSync(root, { recursive: true, force: true });
  }
}

test("fixture starts overlap and both panes accept their first input", { tag: "@platform" }, async () => {
  await withStartBarrier(false, async (root) => {
    const fixture = await startHerdr();
    try {
      for (const [index, pane] of fixture.panes.entries()) {
        expect(fs.existsSync(path.join(root, ["one", "two"][index]! + ".completed"))).toBe(true);
        const marker = `first-input-${index}`;
        execFileSync(fixture.bin, ["pane", "send-text", pane, marker], { env: fixture.env, timeout: 30_000 });
        await expect.poll(() => fs.existsSync(fixture.inputLogs[index]!) ? fs.readFileSync(fixture.inputLogs[index]!, "utf8") : "").toBe(marker);
      }
    } finally {
      fixture.stop();
    }
    expect(fs.existsSync(fixture.root)).toBe(false);
  });
});

test("fixture waits for the sibling start before cleaning up a failed pair", { tag: "@platform" }, async () => {
  await withStartBarrier(true, async (root) => {
    const started = startHerdr();
    // A regression that unexpectedly succeeds must still release its server.
    try {
      await expect(started).rejects.toThrow("fixture agent start failed");
    } finally {
      const result = await Promise.allSettled([started]);
      if (result[0]!.status === "fulfilled") result[0]!.value.stop();
    }
    expect(fs.existsSync(path.join(root, "two.completed"))).toBe(true);
    const fixtureRoot = fs.readFileSync(path.join(root, "fixture-root"), "utf8");
    expect(fs.existsSync(fixtureRoot)).toBe(false);
  });
});

test("registered setup waits for an observed interactive answer before its first input", { tag: "@platform" }, async () => {
  await withHeldLaunch(async (release) => {
    const began = Date.now();
    const fixture = await startHerdr({ readiness: "registered" });
    const setupMs = Date.now() - began;
    let gate: Awaited<ReturnType<typeof herdrGate>> | undefined;
    let pending: Promise<void> | undefined;
    try {
      const registered = fixture.run(["agent", "get", "one"]) as { result: { agent: { interactive_ready?: boolean; launch_pending?: boolean } } };
      expect(registered.result.agent.interactive_ready).not.toBe(true);
      expect(registered.result.agent.launch_pending).toBe(true);
      // Even a fast machine cannot turn registration into permission to send input.
      expect(() => fixture.run(["pane", "send-text", fixture.panes[0], "too early"])).toThrow("requires await fixture.ready()");
      gate = await herdrGate(fixture);
      fixture.env.HERDR_SOCKET_PATH = gate.socket;
      const held = gate.arm("agent.get");
      let completed = false;
      pending = Promise.all([fixture.ready(), fixture.ready()]).then(() => { completed = true; });
      pending.catch(() => undefined);
      await held;
      expect(completed).toBe(false);
      expect(gate.params("agent.get")).toHaveLength(1);
      // Both identities must be read again after an actual unready round.
      const second = gate.arm("agent.get");
      await gate.release();
      await second;
      const nextRound = gate.arm("agent.get");
      await gate.release();
      expect(await Promise.race([nextRound.then(() => "polling"), pending.then(() => "finished")])).toBe("polling");
      expect(completed).toBe(false);
      release();
      await gate.release();
      await pending;
      console.info("fixture startup sample", JSON.stringify({ readiness: "registered", setupMs, readyAfterMs: Date.now() - began }));
      fixture.env.HERDR_SOCKET_PATH = fixture.socket;
      for (const [index, pane] of fixture.panes.entries()) {
        const agent = fixture.run(["agent", "get", pane]) as { result: { agent: { interactive_ready: boolean; launch_pending: boolean } } };
        expect(agent.result.agent.interactive_ready).toBe(true);
        expect(agent.result.agent.launch_pending).not.toBe(true);
        const marker = `registered-first-input-${index}`;
        execFileSync(fixture.bin, ["pane", "send-text", pane, marker], { env: fixture.env, timeout: 30_000 });
        await expect.poll(() => fs.readFileSync(fixture.inputLogs[index]!, "utf8")).toBe(marker);
      }
      await setFixtureLifecycle(fixture, fixture.panes[0], "working");
      await fixture.ready(); // Startup success remains true during a later turn.
    } finally {
      release();
      fixture.env.HERDR_SOCKET_PATH = fixture.socket;
      try { fixture.stop(); } finally {
        await gate?.stop();
        if (pending) await Promise.allSettled([pending]);
      }
    }
  });
});

test("stopping a registered fixture cancels its pending readiness reader", { tag: "@platform" }, async () => {
  const fixture = await startHerdr({ readiness: "registered" });
  let gate: Awaited<ReturnType<typeof herdrGate>> | undefined;
  let pending: Promise<void> | undefined;
  try {
    gate = await herdrGate(fixture);
    fixture.env.HERDR_SOCKET_PATH = gate.socket;
    const held = gate.arm("agent.get");
    pending = fixture.ready();
    pending.catch(() => undefined);
    await held;
    fixture.env.HERDR_SOCKET_PATH = fixture.socket;
    fixture.stop();
    await expect(pending).rejects.toThrow();
    expect(fs.existsSync(fixture.root)).toBe(false);
  } finally {
    fixture.env.HERDR_SOCKET_PATH = fixture.socket;
    try { fixture.stop(); } finally {
      await gate?.stop();
      if (pending) await Promise.allSettled([pending]);
    }
  }
});

test("registered readiness fails when an initial agent has exited", { tag: "@platform" }, async () => {
  const fixture = await startHerdr({ readiness: "registered" });
  try {
    fixture.run(["pane", "close", fixture.panes[0]]);
    await expect(fixture.ready()).rejects.toThrow("agent.get refused");
    expect(() => fixture.run(["agent", "prompt", "two", "too early"])).toThrow("requires await fixture.ready()");
  } finally {
    fixture.stop();
  }
});

test("registered partial failure waits for its held sibling before cleanup", { tag: "@platform" }, async () => {
  await withStartBarrier(false, async (root) => {
    const connect = net.createConnection;
    let release: (() => void) | undefined;
    let held!: () => void;
    let failed!: () => void;
    const siblingHeld = new Promise<void>((resolve) => { held = resolve; });
    const firstFailed = new Promise<void>((resolve) => { failed = resolve; });
    // Intercept only outgoing agent.start bytes. The real pinned server answers
    // the invalid kind; the sibling keeps its unmodified request until released.
    net.createConnection = ((...args: Parameters<typeof net.createConnection>) => {
      const peer = connect(...args);
      const socket = fs.readFileSync(path.join(root, "fixture-socket"), "utf8");
      if (args[0] !== localEndpoint(socket)) return peer;
      const write = peer.write;
      peer.write = ((...parts: Parameters<typeof peer.write>) => {
        const request = JSON.parse(String(parts[0]));
        if (request.method !== "agent.start" || !["one", "two"].includes(request.params.name)) return Reflect.apply(write, peer, parts);
        if (request.params.name === "one") {
          request.params.kind = "invalid-fixture-kind";
          peer.once("data", () => failed());
          parts[0] = `${JSON.stringify(request)}\n`;
        } else {
          release = () => { if (!peer.destroyed) Reflect.apply(write, peer, parts); };
          held();
          return true;
        }
        return Reflect.apply(write, peer, parts);
      }) as typeof peer.write;
      return peer;
    }) as typeof net.createConnection;
    const started = startHerdr({ readiness: "registered" });
    let settled = false;
    started.then(() => { settled = true; }, () => { settled = true; });
    try {
      await Promise.all([siblingHeld, firstFailed]);
      // Drain the real refusal continuation before testing the sibling barrier.
      await new Promise<void>((resolve) => setImmediate(resolve));
      expect(settled).toBe(false);
      const fixtureRoot = fs.readFileSync(path.join(root, "fixture-root"), "utf8");
      expect(fs.existsSync(fixtureRoot)).toBe(true);
      release!();
      release = undefined;
      await expect(started).rejects.toThrow("fixture agent start failed");
      expect(fs.existsSync(fixtureRoot)).toBe(false);
    } finally {
      net.createConnection = connect;
      release?.();
      const result = await Promise.allSettled([started]);
      if (result[0]!.status === "fulfilled") result[0]!.value.stop();
    }
  });
});
