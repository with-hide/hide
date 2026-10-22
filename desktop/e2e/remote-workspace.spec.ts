// Real SSH-origin Workspace CLI acceptance. Run with an isolated sshd whose
// port, key and known_hosts are supplied in HIDE_E2E_SSH_* and whose sessions
// get the private HOME desktop/e2e/device-home.ts describes, because
// connecting installs Hide's kit there; the two Herdr servers, daemon, helper
// install and desktop profile remain private.
// HIDE_E2E_SSH_PID plus HIDE_E2E_SSH_CONFIG enable the stalled-handshake
// check only for an identified, test-owned sshd listener.

import { expect, type Page } from "@playwright/test";
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import http from "node:http";
import https from "node:https";
import net from "node:net";
import type { AddressInfo } from "node:net";
import type { Duplex } from "node:stream";
import path from "node:path";
import { startHerdr, type HerdrFixture } from "../../web/e2e/herdr-fixture";
import { enterWorkspace, showDocuments } from "../../web/e2e/wire";
import { browserPartition } from "../src/main/browserSync";
import { claudeSettings, deviceBridges, deviceHome, proveDeviceHome, readSettings, resetDeviceHome, writeSshConfig } from "./device-home";
import { endChild, HIDE_CLI, hostLog, isolate, launch, nodeOf, test, type Isolated } from "./fixture";

const HOOK_CLI = path.join(path.dirname(HIDE_CLI), "hide-agent-hooks");

test.describe.configure({ timeout: 300_000 });
test.skip(!process.env.HIDE_E2E_SSH_PORT, "an isolated SSH server is required");

/** Shows the Workspace in front's View areas, bringing the documents up if a background open left the lane alone. */
async function showViews(page: Page): Promise<void> {
  await showDocuments(page);
}

function quote(value: string): string { return `'${value.replaceAll("'", "'\\''")}'`; }

async function canBindLoopback(port: number, host: string): Promise<boolean> {
  return new Promise((resolve) => {
    const listener = net.createServer();
    listener.once("error", () => resolve(false));
    listener.listen(port, host, () => listener.close(() => resolve(true)));
  });
}

function changedCheckout(herdr: HerdrFixture, filename: string): string {
  const checkout = path.join(herdr.root, "fixture");
  const file = path.join(checkout, filename);
  fs.writeFileSync(file, "Baseline\n");
  for (const args of [["init", "-q"], ["add", filename], ["-c", "user.name=Fixture", "-c", "user.email=fixture@example.com", "commit", "-qm", "Fixture baseline"]]) {
    const result = spawnSync("git", ["-C", checkout, ...args], { encoding: "utf8", timeout: 10_000 });
    expect(result.status, result.stderr).toBe(0);
  }
  fs.appendFileSync(file, "Changed\n");
  return file;
}

/** Live bridge folders: each holds the socket a device pane bootstraps through. */
/**
 * The pane services answering in `bridge`, found as a pane's `hide` finds
 * them: by a socket that accepts a connection. A node that was killed leaves
 * its folder until a later start removes it.
 */
function liveBridges(bridge: string): string[] {
  if (!fs.existsSync(bridge)) return [];
  return fs.readdirSync(bridge).filter((name) => answers(path.join(bridge, name, "bootstrap.sock")));
}

function answers(socket: string): boolean {
  if (!fs.existsSync(socket)) return false;
  const probe = spawnSync(process.execPath, [
    "-e", "require('node:net').connect(process.argv[1]).on('connect', () => process.exit(0)).on('error', () => process.exit(1))", socket,
  ], { timeout: 5_000 });
  return probe.status === 0;
}

/** The daemon's diagnostic records, which it writes to stderr and its Logs file alike. */
function daemonEvents(log: string): { kind?: string; reason?: string; generation?: number; device_id?: string }[] {
  return fs.readFileSync(log, "utf8").split("\n").filter((line) => line.startsWith("{")).map((line) => JSON.parse(line));
}

async function sendFrame(page: Page, state: { port: number; token: string }, frame: Record<string, unknown>): Promise<void> {
  await page.evaluate(async ({ port, token, frame }) => {
    await new Promise<void>((resolve, reject) => {
      const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`);
      const timer = setTimeout(() => reject(new Error("frame timed out")), 10_000);
      ws.onerror = () => { clearTimeout(timer); reject(new Error("frame socket failed")); };
      ws.onopen = () => ws.send(JSON.stringify({ token, schema_version: 2 }));
      ws.onmessage = () => {
        ws.send(JSON.stringify({ schema_version: 2, ...frame }));
        clearTimeout(timer); ws.close(); resolve();
      };
    });
  }, { ...state, frame });
}

/** The operator brings `device` to the front as the device picker does, and the screen shows the SSH device's Workspace or not. */
async function showDevice(page: Page, state: { port: number; token: string }, device: string): Promise<void> {
  await sendFrame(page, state, { kind: "focus_device", payload: { device_id: device } });
  const screen = expect.poll(() => page.locator("[data-workspace-screen]").getAttribute("data-workspace-screen"), { timeout: 20_000 });
  await (device === "ssh-e2e" ? screen.toContain("ssh-e2e") : screen.not.toContain("ssh-e2e"));
}

/**
 * A device pane runs the `hide` Hide installed and linked on that device,
 * as an operator typing `hide` there would; a local pane runs this build's.
 */
async function commandFromPane(herdr: HerdrFixture, run: Isolated, bridge: string | null, args: string[], label: string) {
  const result = path.join(herdr.root, `${label}.json`);
  const exit = path.join(herdr.root, `${label}.exit`);
  const variables = bridge
    ? `HIDE_STATE_DIR=${quote(path.dirname(bridge))}`
    : `HIDE_STATE_DIR=${quote(run.env.HIDE_STATE_DIR!)}`;
  const cli = bridge ? path.join(run.env.HIDE_HOST_CLI_DIR!, "hide") : HIDE_CLI;
  const command = `${variables} ${[cli, ...args].map(quote).join(" ")} > ${quote(result)}; printf '%s' "$?" > ${quote(exit)}\n`;
  const sent = spawnSync(herdr.bin, ["pane", "send-text", herdr.panes[0], command], { env: herdr.env, encoding: "utf8", timeout: 10_000 });
  expect(sent.status, sent.stderr).toBe(0);
  await expect.poll(() => fs.existsSync(exit), { timeout: 30_000 }).toBe(true);
  return { status: Number(fs.readFileSync(exit, "utf8")), answer: JSON.parse(fs.readFileSync(result, "utf8").trim().split("\n").at(-1) || "{}") as Record<string, unknown> };
}

async function hookFromPane(herdr: HerdrFixture, stateDir: string, runtime: "claude-code" | "codex", label: string) {
  const result = path.join(herdr.root, `${label}.json`);
  const exit = path.join(herdr.root, `${label}.exit`);
  const command = `printf '{}' | HIDE_STATE_DIR=${quote(stateDir)} ${quote(HOOK_CLI)} hook --runtime ${runtime} --event SessionStart --memory-injection --source hide-subagents@6 > ${quote(result)}; printf '%s' "$?" > ${quote(exit)}\n`;
  const sent = spawnSync(herdr.bin, ["pane", "send-text", herdr.panes[0], command], { env: herdr.env, encoding: "utf8", timeout: 10_000 });
  expect(sent.status, sent.stderr).toBe(0);
  await expect.poll(() => fs.existsSync(exit), { timeout: 30_000 }).toBe(true);
  const output = JSON.parse(fs.readFileSync(result, "utf8")) as { hookSpecificOutput: { additionalContext: string } };
  return { status: Number(fs.readFileSync(exit, "utf8")), context: output.hookSpecificOutput.additionalContext };
}

test("remote pane CLI reaches its own Workspace over SSH and leaves the local Workspace in front", async () => {
  const local = await startHerdr({ agents: false });
  const remote = await startHerdr({ agents: false });
  const localFile = changedCheckout(local, "local-change.md");
  const remoteFile = changedCheckout(remote, "보고서-mixed-2026.md");
  const run = isolate(local, "ssh");
  const helper = path.join(run.root, "remote-helper");
  run.env.HIDE_HOST_HELPER_ROOT = helper;
  // The device's `hide` link goes here, never the account's own ~/.local/bin.
  run.env.HIDE_HOST_CLI_DIR = path.join(run.root, "remote-bin");
  // Two aliases for the one isolated server: a second device registers under
  // the second, with its own Herdr server, id and connections.
  writeSshConfig(run.env.HOME!, ["isolated-workspace", "isolated-workspace-2"]);
  const deviceAccount = deviceHome();
  const bridge = deviceBridges(deviceAccount);
  proveDeviceHome(run.env, "isolated-workspace", deviceAccount);
  resetDeviceHome(deviceAccount);
  const daemonLog = path.join(run.root, "daemon.log");
  const daemonOutput = fs.openSync(daemonLog, "w");
  const daemon = spawn(path.join(path.dirname(HIDE_CLI), "hided"), [], {
    env: run.env, stdio: ["ignore", daemonOutput, daemonOutput],
  });
  fs.closeSync(daemonOutput);
  let app: Awaited<ReturnType<typeof launch>>["app"] | undefined;
  let devServer: http.Server | undefined;
  let tlsServer: https.Server | undefined;
  let tlsPartition: string | undefined;
  let collisionServer: http.Server | undefined;
  let decoyServer: http.Server | undefined;
  let egressServer: http.Server | undefined;
  let twinServer: http.Server | undefined;
  let second: HerdrFixture | undefined;
  const upgraded = new Set<Duplex>();
  try {
    await expect.poll(() => run.hide(["status", "--json"]).stdout.includes('"running":true'), { timeout: 30_000 }).toBe(true);
    ({ app } = await launch(run.env));
    const page = await app.firstWindow();
    await enterWorkspace(page, "fixture");
    for (const [args, label] of [
      [["workspace", "info"], "local-info"],
      [["file", "open", localFile], "local-file"],
      [["diff", "open", localFile], "local-diff"],
    ] as const) {
      const result = await commandFromPane(local, run, null, [...args], label);
      expect(result.status, JSON.stringify(result.answer)).toBe(0);
      expect(result.answer).toMatchObject({ ok: true, result: { context: { device_id: nodeOf(run.env) } } });
    }
    for (const runtime of ["claude-code", "codex"] as const) {
      const localContext = await hookFromPane(local, run.env.HIDE_STATE_DIR!, runtime, `local-${runtime}-hook`);
      expect(localContext.status).toBe(0);
      expect(localContext.context).toContain("Hide Workspace control is available for this session's checkout");
      expect(localContext.context).toContain("file open <path>");
      expect(localContext.context).toContain("browser open <url-or-path>");
      const remoteBeforeConnection = await hookFromPane(remote, path.dirname(bridge), runtime, `disconnected-${runtime}-hook`);
      expect(remoteBeforeConnection.status).toBe(0);
      expect(remoteBeforeConnection.context).not.toContain("Hide Workspace control is available");
    }
    const plain = spawnSync(HOOK_CLI, ["hook", "--runtime", "codex", "--event", "SessionStart", "--memory-injection", "--source", "hide-subagents@6"], {
      env: { ...local.env, HERDR_PANE_ID: "", HIDE_STATE_DIR: run.env.HIDE_STATE_DIR! },
      input: "{}", encoding: "utf8", timeout: 10_000,
    });
    expect(plain.status).toBe(0);
    expect(plain.stdout).not.toContain("Hide Workspace control is available");
    // Outside any pane but inside a registered checkout (a Codex shared daemon,
    // a plain terminal): the daemon binds the caller by its cwd and the hook
    // names that checkout.
    const noPaneEnv: NodeJS.ProcessEnv = { ...local.env, HIDE_STATE_DIR: run.env.HIDE_STATE_DIR! };
    delete noPaneEnv.HERDR_PANE_ID;
    const plainInCheckout = spawnSync(HOOK_CLI, ["hook", "--runtime", "codex", "--event", "SessionStart", "--memory-injection", "--source", "hide-subagents@6"], {
      cwd: path.join(local.root, "fixture"),
      env: noPaneEnv,
      input: "{}", encoding: "utf8", timeout: 10_000,
    });
    expect(plainInCheckout.status).toBe(0);
    expect(plainInCheckout.stdout).toContain("Hide Workspace control is available for this session's checkout");
    expect(plainInCheckout.stdout).toContain(fs.realpathSync(path.join(local.root, "fixture")));
    const state = JSON.parse(fs.readFileSync(path.join(run.env.HIDE_STATE_DIR!, "hided.json"), "utf8")) as { port: number; token: string };
    await page.evaluate(async ({ port, token, socket }) => {
      await new Promise<void>((resolve, reject) => {
        const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`);
        const timer = setTimeout(() => reject(new Error("device registration timed out")), 10_000);
        ws.onerror = () => { clearTimeout(timer); reject(new Error("device registration socket failed")); };
        ws.onopen = () => ws.send(JSON.stringify({ token, schema_version: 2 }));
        ws.onmessage = () => {
          ws.send(JSON.stringify({ schema_version: 2, kind: "register_device", payload: {
            id: "ssh-e2e", label: "SSH fixture", ssh_alias: "isolated-workspace",
            herdr_socket_path: socket, host_consent: true,
          } }));
          clearTimeout(timer);
          ws.close();
          resolve();
        };
      });
    }, { port: state.port, token: state.token, socket: remote.socket });
    await expect.poll(() => liveBridges(bridge).length, { timeout: 60_000 }).toBe(1);
    // The connection installed Hide's kit on the device: `hide` is linked
    // where the device's panes find it, through the helper root's `current`.
    await expect.poll(() => daemonEvents(daemonLog).some((line) => line.kind === "apply.completed" && line.device_id === "ssh-e2e"), { timeout: 120_000 }).toBe(true);
    const deviceCli = path.join(run.env.HIDE_HOST_CLI_DIR, "hide");
    expect(fs.readlinkSync(deviceCli)).toBe(path.join(fs.realpathSync(helper), "current", "hide"));
    expect(fs.realpathSync(deviceCli).startsWith(fs.realpathSync(helper) + path.sep)).toBe(true);
    const sessionReferences: string[] = [];
    for (const runtime of ["claude-code", "codex"] as const) {
      const session = await hookFromPane(remote, path.dirname(bridge), runtime, `remote-${runtime}-hook`);
      expect(session.status).toBe(0);
      expect(session.context).toContain("Hide Workspace control is available for this session's checkout");
      expect(session.context).toContain("browser open <url-or-path>");
      const reference = session.context.match(/HIDE_CAP_REF='([^']+)'/)?.[1];
      expect(reference).toBeTruthy();
      sessionReferences.push(reference!);
      const detachedEnv: NodeJS.ProcessEnv = { ...run.env, HIDE_CAP_REF: reference! };
      delete detachedEnv.HERDR_PANE_ID;
      const detached = spawnSync(HIDE_CLI, ["workspace", "info"], {
        env: detachedEnv,
        encoding: "utf8", timeout: 10_000,
      });
      expect(detached.status, `${detached.stderr} ${detached.stdout}`).toBe(0);
      expect(JSON.parse(detached.stdout)).toMatchObject({ ok: true, result: { context: { device_id: "ssh-e2e" } } });
    }
    expect(new Set(sessionReferences).size).toBe(1);
    const stressScript = path.join(remote.root, "repeat-session-start.sh");
    const stressOutput = path.join(remote.root, "repeat-session-start.out");
    const stressExit = path.join(remote.root, "repeat-session-start.exit");
    const stressError = path.join(remote.root, "repeat-session-start.err");
    fs.writeFileSync(stressScript, [
      "#!/bin/bash", "set -euo pipefail", "for i in $(seq 1 65); do",
      `  context=$(printf '{}' | ${quote(HOOK_CLI)} hook --runtime codex --event SessionStart --memory-injection --source hide-subagents@6 | jq -er .hookSpecificOutput.additionalContext)`,
      `  reference=$(printf '%s' "$context" | sed -n "s/.*HIDE_CAP_REF='\\([^']*\\)'.*/\\1/p")`,
      "  test -n \"$reference\"",
      `  HIDE_CAP_REF="$reference" ${quote(HIDE_CLI)} workspace info >/dev/null`,
      "  printf '%s\\n' \"$reference\"", "done", "",
    ].join("\n"), { mode: 0o700 });
    const stressCommand = `HIDE_STATE_DIR=${quote(path.dirname(bridge))} bash ${quote(stressScript)} > ${quote(stressOutput)} 2> ${quote(stressError)}; printf '%s' "$?" > ${quote(stressExit)}\n`;
    const stressSent = spawnSync(remote.bin, ["pane", "send-text", remote.panes[0], stressCommand], { env: remote.env, encoding: "utf8", timeout: 10_000 });
    expect(stressSent.status, stressSent.stderr).toBe(0);
    await expect.poll(() => fs.existsSync(stressExit), { timeout: 180_000 }).toBe(true);
    expect(fs.readFileSync(stressExit, "utf8"), `${fs.readFileSync(stressError, "utf8")}\n${fs.readFileSync(stressOutput, "utf8")}`).toBe("0");
    const repeatedReferences = fs.readFileSync(stressOutput, "utf8").trim().split("\n");
    expect(repeatedReferences).toHaveLength(65);
    expect(new Set(repeatedReferences)).toEqual(new Set(sessionReferences));
    const info = await commandFromPane(remote, run, bridge, ["workspace", "info"], "remote-info");
    expect(info.status, JSON.stringify(info.answer)).toBe(0);
    expect(info.answer).toMatchObject({ ok: true, result: { context: { device_id: "ssh-e2e", checkout_path: fs.realpathSync(path.join(remote.root, "fixture")) } } });

    // The device's node ends, which closes its link and revokes what it
    // vouched for, and the device comes back on a new connection
    // (Settings > Retry) whose node opens a new pane socket; the pane
    // reaches its Workspace again through the same installed command.
    const firstBridges = liveBridges(bridge);
    const serving = spawnSync("pgrep", ["-f", `${path.basename(run.root)}/remote-helper/.*/hided node serve`], { encoding: "utf8" })
      .stdout.trim().split("\n").filter(Boolean);
    expect(serving).toHaveLength(1);
    process.kill(Number(serving[0]), "SIGTERM");
    await expect.poll(() => daemonEvents(daemonLog).some((line) => line.kind === "host.closed"), { timeout: 30_000 }).toBe(true);
    await sendFrame(page, state, { kind: "device_host_retry", payload: { device_id: "ssh-e2e" } });
    await expect.poll(() => liveBridges(bridge).filter((name) => !firstBridges.includes(name)).length, { timeout: 60_000 }).toBe(1);
    const reconnected = await commandFromPane(remote, run, bridge, ["workspace", "info"], "remote-info-reconnected");
    expect(reconnected.status, JSON.stringify(reconnected.answer)).toBe(0);
    expect(reconnected.answer).toMatchObject({ ok: true, result: { context: { device_id: "ssh-e2e" } } });
    // Each connection started the device's pane service on its own link.
    expect(daemonEvents(daemonLog).filter((line) => line.kind === "host.panes_started").length).toBe(2);
    // The same records reach the Logs file a detached daemon writes.
    const coreLog = fs.readFileSync(path.join(run.env.HIDE_STATE_DIR!, "Logs", "core.jsonl"), "utf8");
    expect(coreLog).toContain('"kind":"host.closed"');
    expect(coreLog).toContain('"kind":"host.panes_started"');

    // A second device at once. Each gets its own route; a pane on either
    // reaches only its own device's Workspace, even though both bridges sit
    // in the one folder here; each device's loopback page gets its own
    // forward on this Mac, and neither device's page is served for the other;
    // removing one device leaves the other's route and commands untouched.
    second = await startHerdr({ agents: false });
    const secondCheckout = fs.realpathSync(path.join(second.root, "fixture"));
    const firstCheckout = fs.realpathSync(path.join(remote.root, "fixture"));
    await sendFrame(page, state, { kind: "register_device", payload: {
      id: "ssh-e2e-2", label: "SSH fixture 2", ssh_alias: "isolated-workspace-2",
      herdr_socket_path: second.socket, host_consent: true,
    } });
    await expect.poll(() => liveBridges(bridge).length, { timeout: 60_000 }).toBe(2);
    const secondInfo = await commandFromPane(second, run, bridge, ["workspace", "info"], "second-info");
    expect(secondInfo.status, JSON.stringify(secondInfo.answer)).toBe(0);
    expect(secondInfo.answer).toMatchObject({ ok: true, result: { context: { device_id: "ssh-e2e-2", checkout_path: secondCheckout } } });
    const firstInfo = await commandFromPane(remote, run, bridge, ["workspace", "info"], "first-info-beside-second");
    expect(firstInfo.status, JSON.stringify(firstInfo.answer)).toBe(0);
    expect(firstInfo.answer).toMatchObject({ ok: true, result: { context: { device_id: "ssh-e2e", checkout_path: firstCheckout } } });
    twinServer = http.createServer((_request, response) => {
      response.writeHead(200, { "content-type": "text/html; charset=utf-8" });
      response.end("<!doctype html><title>Twin</title><h1>twin</h1>");
    });
    await new Promise<void>((resolve) => twinServer!.listen(0, "127.0.0.1", resolve));
    const twinPort = (twinServer.address() as AddressInfo).port;
    const openTwin = async (herdr: HerdrFixture, label: string) => {
      const opened = await commandFromPane(herdr, run, bridge, ["browser", "open", `http://localhost:${twinPort}/twin.html`], label);
      expect(opened.status, JSON.stringify(opened.answer)).toBe(0);
      return opened.answer.result as { view_id: string; load: number };
    };
    const firstTwin = await openTwin(remote, "twin-first");
    const secondTwin = await openTwin(second, "twin-second");
    const ownerPid = app.process().pid;
    const routeFor = (device: string, checkout: string, view: { view_id: string; load: number }) =>
      fetch(`http://127.0.0.1:${state.port}/browser-route`, {
        method: "POST", headers: { Authorization: `Bearer ${state.token}`, "Content-Type": "application/json" },
        body: JSON.stringify({ device_id: device, checkout_path: checkout, id: view.view_id, load: view.load, owner_pid: ownerPid }),
      });
    const routes = await Promise.all([
      routeFor("ssh-e2e", firstCheckout, firstTwin),
      routeFor("ssh-e2e-2", secondCheckout, secondTwin),
    ]);
    expect(routes.map((response) => response.status)).toEqual([200, 200]);
    const [firstRoute, secondRoute] = await Promise.all(routes.map((response) => response.json() as Promise<{ url: string }>));
    const ports = [firstRoute, secondRoute].map((route) => new URL(route.url).port);
    expect(new Set(ports).size).toBe(2);
    expect(ports).not.toContain(String(twinPort));
    for (const route of [firstRoute, secondRoute]) {
      expect(await (await fetch(route.url, { signal: AbortSignal.timeout(5_000) })).text()).toContain("<h1>twin</h1>");
    }
    // A View is only ever routed through the device that holds it.
    expect((await routeFor("ssh-e2e", secondCheckout, secondTwin)).status).not.toBe(200);
    expect((await commandFromPane(remote, run, bridge, ["view", "close", firstTwin.view_id], "twin-first-close")).status).toBe(0);
    expect((await commandFromPane(second, run, bridge, ["view", "close", secondTwin.view_id], "twin-second-close")).status).toBe(0);
    const beforeRemoval = daemonEvents(daemonLog).length;
    await sendFrame(page, state, { kind: "remove_device", payload: { device_id: "ssh-e2e-2" } });
    await expect.poll(() => liveBridges(bridge).length, { timeout: 30_000 }).toBe(1);
    // Both registrations reach this one account, so the kit stays for the survivor.
    expect(daemonEvents(daemonLog).slice(beforeRemoval).find((line) => line.kind === "device.kit_left" && line.device_id === "ssh-e2e-2")?.reason)
      .toBe("another registered device reaches the same account on that machine");
    expect(fs.readlinkSync(deviceCli)).toBe(path.join(fs.realpathSync(helper), "current", "hide"));
    expect(JSON.stringify(readSettings(claudeSettings(deviceAccount)))).toContain(path.join(fs.realpathSync(helper), "current", "hide-agent-hooks"));
    const survivor = await commandFromPane(remote, run, bridge, ["workspace", "info"], "first-info-after-removal");
    expect(survivor.status, JSON.stringify(survivor.answer)).toBe(0);
    expect(survivor.answer).toMatchObject({ ok: true, result: { context: { device_id: "ssh-e2e" } } });
    expect(daemonEvents(daemonLog).slice(beforeRemoval).filter((line) =>
      (line as { device_id?: string }).device_id === "ssh-e2e" && /^route\.(stopped|failed|closed)$/.test(line.kind ?? ""))).toEqual([]);
    const opened = await commandFromPane(remote, run, bridge, ["file", "open", remoteFile], "remote-file");
    expect(opened.status, JSON.stringify(opened.answer)).toBe(0);
    expect(opened.answer).toMatchObject({ ok: true, result: { context: { device_id: "ssh-e2e" }, changed: true } });
    const requestId = opened.answer.request_id as string;
    expect(requestId).toMatch(/^\d+-[A-Za-z0-9]+$/);
    const repeated = await commandFromPane(remote, run, bridge, ["file", "open", remoteFile, "--request-id", requestId], "remote-file-repeat");
    expect(repeated.status).toBe(0);
    expect(repeated.answer).toEqual(opened.answer);
    const diff = await commandFromPane(remote, run, bridge, ["diff", "open", remoteFile], "remote-diff");
    expect(diff.status, JSON.stringify(diff.answer)).toBe(0);
    expect(diff.answer).toMatchObject({ ok: true, result: { context: { device_id: "ssh-e2e" }, changed: true } });
    const views = await commandFromPane(remote, run, bridge, ["view", "list"], "remote-views");
    expect(views.status, JSON.stringify(views.answer)).toBe(0);
    const rows = ((views.answer.result as { views: { view_id: string; area_id: string; kind: string; target: string }[] }).views);
    const fileView = rows.find((view) => view.target === fs.realpathSync(remoteFile) && view.kind === "file");
    const diffView = rows.find((view) => view.target === fs.realpathSync(remoteFile) && view.kind === "diff");
    expect(fileView).toBeTruthy();
    expect(diffView).toBeTruthy();
    expect((await commandFromPane(remote, run, bridge, ["view", "select", fileView!.view_id], "remote-select")).status).toBe(0);
    const split = await commandFromPane(remote, run, bridge, ["view", "split", diffView!.view_id, "--area", fileView!.area_id, "--edge", "right"], "remote-split");
    expect(split.status, JSON.stringify(split.answer)).toBe(0);
    const secondArea = (split.answer.result as { area_id: string }).area_id;
    expect(secondArea).not.toBe(fileView!.area_id);
    expect((await commandFromPane(remote, run, bridge, ["view", "move", fileView!.view_id, "--area", secondArea, "--index", "0"], "remote-move")).status).toBe(0);
    expect((await commandFromPane(remote, run, bridge, ["view", "close", diffView!.view_id], "remote-diff-close")).status).toBe(0);
    await expect(page.locator("[data-workspace-screen]")).toBeVisible();
    expect(await page.locator("[data-workspace-screen]").getAttribute("data-workspace-screen")).not.toContain("ssh-e2e");

    const tlsKey = path.join(run.root, "localhost-key.pem");
    const tlsCert = path.join(run.root, "localhost-cert.pem");
    const certificate = spawnSync("openssl", ["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1", "-subj", "/CN=localhost", "-addext", "subjectAltName=DNS:localhost", "-keyout", tlsKey, "-out", tlsCert], { encoding: "utf8", timeout: 10_000 });
    expect(certificate.status, certificate.stderr).toBe(0);
    tlsServer = https.createServer({ key: fs.readFileSync(tlsKey), cert: fs.readFileSync(tlsCert) }, (_request, response) => response.end("remote-secure"));
    await new Promise<void>((resolve) => tlsServer!.listen(0, "127.0.0.1", resolve));
    const tlsPort = (tlsServer.address() as AddressInfo).port;
    const remoteWorkspace = `ssh-e2e\u0000${fs.realpathSync(path.join(remote.root, "fixture"))}`;
    tlsPartition = browserPartition(remoteWorkspace, `https://localhost:${tlsPort}/secure`, nodeOf(run.env));
    await app.evaluate(({ session }, { partition, certificate }) => {
      session.fromPartition(partition).setCertificateVerifyProc((request, callback) => {
        callback(request.hostname === "localhost" && request.certificate.data.trim() === certificate.trim() ? 0 : -3);
      });
    }, { partition: tlsPartition, certificate: fs.readFileSync(tlsCert, "utf8") });
    const openedTls = await commandFromPane(remote, run, bridge, ["browser", "open", `HTTPS://localhost:${tlsPort}/secure`], "remote-tls");
    expect(openedTls.status, JSON.stringify(openedTls.answer)).toBe(0);
    const tlsView = openedTls.answer.result as { view_id: string; load: number };
    const tlsRouteResponse = await fetch(`http://127.0.0.1:${state.port}/browser-route`, {
      method: "POST", headers: { Authorization: `Bearer ${state.token}`, "Content-Type": "application/json" },
      body: JSON.stringify({ device_id: "ssh-e2e", checkout_path: fs.realpathSync(path.join(remote.root, "fixture")), id: tlsView.view_id, load: tlsView.load, owner_pid: app.process().pid }),
    });
    expect(tlsRouteResponse.status).toBe(200);
    const tlsRoute = await tlsRouteResponse.json() as { url: string; source_url: string };
    expect(new URL(tlsRoute.url).protocol).toBe("https:");
    expect(new URL(tlsRoute.url).hostname).toBe("localhost");
    const ipv6Collision = await new Promise<string>((resolve) => {
      const decoy = net.createServer();
      decoy.once("error", (error: NodeJS.ErrnoException) => resolve(error.code ?? "unknown"));
      decoy.listen(Number(new URL(tlsRoute.url).port), "::1", () => {
        decoy.close();
        resolve("unexpected-bind");
      });
    });
    expect(ipv6Collision).toBe("EADDRINUSE");
    const secureBody = await new Promise<string>((resolve, reject) => {
      const request = https.get(tlsRoute.url, { ca: fs.readFileSync(tlsCert), timeout: 5_000 }, (response) => {
        let body = "";
        response.setEncoding("utf8");
        response.on("data", (chunk: string) => { body += chunk; });
        response.on("end", () => resolve(body));
      });
      request.on("error", reject);
    });
    expect(secureBody).toBe("remote-secure");

    devServer = http.createServer((request, response) => {
      const body = request.url === "/remote.html"
        ? `<!doctype html><title>Remote dev</title><h1 id="origin">Remote dev</h1><p id="live">waiting</p><script>new WebSocket('ws://' + location.host + '/live').onmessage = e => document.getElementById('live').textContent = e.data</script>`
        : request.url === "/child.html" ? "<!doctype html><title>Remote child</title><h1>Remote child</h1>"
        : null;
      response.writeHead(body ? 200 : 404, { "content-type": "text/html; charset=utf-8" });
      response.end(body ?? "missing");
    });
    devServer.on("upgrade", (request, socket) => {
      const key = request.headers["sec-websocket-key"];
      if (request.url !== "/live" || typeof key !== "string") return socket.destroy();
      upgraded.add(socket);
      socket.on("close", () => upgraded.delete(socket));
      const accept = createHash("sha1").update(`${key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`).digest("base64");
      socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
      const payload = Buffer.from("remote-live");
      socket.write(Buffer.concat([Buffer.from([0x81, payload.length]), payload]));
    });
    await new Promise<void>((resolve) => devServer!.listen(0, "127.0.0.1", resolve));
    const remotePort = (devServer.address() as AddressInfo).port;
    const openedDev = await commandFromPane(remote, run, bridge, ["browser", "open", `http://localhost:${remotePort}/remote.html`], "remote-dev");
    expect(openedDev.status, JSON.stringify(openedDev.answer)).toBe(0);
    expect(await page.locator("[data-workspace-screen]").getAttribute("data-workspace-screen")).not.toContain("ssh-e2e");

    await showDevice(page, state, "ssh-e2e");
    expect((await commandFromPane(remote, run, bridge, ["view", "select", tlsView.view_id], "remote-tls-select")).status).toBe(0);
    await showViews(page);
    await expect.poll(async () => app!.evaluate(async ({ BrowserWindow }, routeUrl) => {
      const child = BrowserWindow.getAllWindows()[0]?.contentView.children.find((entry) =>
        (entry as { webContents?: Electron.WebContents }).webContents?.getURL() === routeUrl);
      return child ? (child as unknown as { webContents: Electron.WebContents }).webContents.executeJavaScript("document.body.textContent") as Promise<string> : null;
    }, tlsRoute.url), { timeout: 30_000 }).toContain("remote-secure");
    // This certificate names localhost, not the numeric host; the same device partition must reject it.
    const beforeInvalidCertificate = hostLog(run.env).length;
    const invalidTls = await commandFromPane(remote, run, bridge, ["browser", "open", `https://127.0.0.1:${tlsPort}/invalid`], "remote-tls-invalid");
    expect(invalidTls.status, JSON.stringify(invalidTls.answer)).toBe(0);
    const invalidTlsView = (invalidTls.answer.result as { view_id: string }).view_id;
    await expect(page.locator(`[data-browser-display="${invalidTlsView}"] [data-area-empty="browser-failed"]`)).toBeVisible();
    await expect.poll(() => hostLog(run.env).slice(beforeInvalidCertificate).some((line) =>
      line.event === "browser.load_failed" && String(line.description).startsWith("ERR_CERT_"))).toBe(true);
    expect((await commandFromPane(remote, run, bridge, ["view", "close", invalidTlsView], "remote-tls-invalid-close")).status).toBe(0);
    expect((await commandFromPane(remote, run, bridge, ["view", "close", tlsView.view_id], "remote-tls-close")).status).toBe(0);
    await expect.poll(async () => Promise.all(["127.0.0.1", "::1"].map((host) =>
      canBindLoopback(Number(new URL(tlsRoute.url).port), host))), { timeout: 10_000 }).toEqual([true, true]);
    await app.evaluate(({ session }, partition) => session.fromPartition(partition).setCertificateVerifyProc(null), tlsPartition);
    tlsPartition = undefined;
    expect((await commandFromPane(remote, run, bridge, ["view", "select", (openedDev.answer.result as { view_id: string }).view_id], "remote-dev-select")).status).toBe(0);
    const native = async () => app!.evaluate(async ({ BrowserWindow }, sourcePort) => {
      const child = BrowserWindow.getAllWindows()[0]?.contentView.children.find((entry) =>
        (entry as { webContents?: Electron.WebContents }).webContents?.getTitle() === "Remote dev"
        && new URL((entry as unknown as { webContents: Electron.WebContents }).webContents.getURL()).port !== sourcePort);
      if (!child) return null;
      const contents = (child as unknown as { webContents: Electron.WebContents }).webContents;
      return { url: contents.getURL(), live: await contents.executeJavaScript("document.getElementById('live')?.textContent") as string };
    }, String(remotePort));
    await expect.poll(async () => (await native())?.live, { timeout: 30_000 }).toBe("remote-live");
    await app.evaluate(async ({ BrowserWindow }) => {
      const contents = (BrowserWindow.getAllWindows()[0]?.contentView.children.find((entry) =>
        (entry as { webContents?: Electron.WebContents }).webContents?.getTitle() === "Remote dev") as { webContents: Electron.WebContents } | undefined)?.webContents;
      if (!contents) throw new Error("Remote page is missing for cookie isolation");
      await contents.executeJavaScript("document.cookie = 'device=remote; path=/';");
    });
    const localBrowser = await commandFromPane(local, run, null, ["browser", "open", `http://localhost:${remotePort}/remote.html`], "local-dev-cookie");
    expect(localBrowser.status).toBe(0);
    await showDevice(page, state, nodeOf(run.env));
    expect((await commandFromPane(local, run, null, ["view", "select", (localBrowser.answer.result as { view_id: string }).view_id], "local-dev-select")).status).toBe(0);
    await showViews(page);
    await expect.poll(async () => app!.evaluate(async ({ BrowserWindow }) => {
      const pages = BrowserWindow.getAllWindows()[0]?.contentView.children.filter((entry) =>
        (entry as { webContents?: Electron.WebContents }).webContents?.getTitle() === "Remote dev") as unknown as { webContents: Electron.WebContents }[] | undefined;
      if (pages?.length !== 2) return null;
      return Promise.all(pages.map((entry) => entry.webContents.executeJavaScript("document.cookie") as Promise<string>));
    }), { timeout: 20_000 }).toEqual(expect.arrayContaining(["device=remote", ""]));
    await showDevice(page, state, "ssh-e2e");
    const devViewId = (openedDev.answer.result as { view_id: string }).view_id;
    const status = await commandFromPane(remote, run, bridge, ["view", "status", devViewId], "remote-dev-status");
    expect(status.status, JSON.stringify(status.answer)).toBe(0);
    expect(status.answer).toMatchObject({ ok: true, view: { page: { state: "loaded" } } });
    const waited = await commandFromPane(remote, run, bridge, ["browser", "open", `http://localhost:${remotePort}/remote.html`, "--wait"], "remote-dev-wait");
    expect(waited.status, JSON.stringify(waited.answer)).toBe(0);
    expect(waited.answer).toMatchObject({ ok: true, page: { state: "loaded" } });
    await expect.poll(async () => (await native())?.live, { timeout: 20_000 }).toBe("remote-live");
    const routed = (await native())!.url;
    expect(new URL(routed).port).not.toBe(String(remotePort));
    expect(new URL(routed).hostname).toBe("127.0.0.1");
    await app.evaluate(async ({ BrowserWindow }, sourcePort) => {
      const child = BrowserWindow.getAllWindows()[0]?.contentView.children.find((entry) =>
        (entry as { webContents?: Electron.WebContents }).webContents?.getTitle() === "Remote dev"
        && new URL((entry as unknown as { webContents: Electron.WebContents }).webContents.getURL()).port !== sourcePort);
      if (!child) throw new Error("Remote page is missing for its popup");
      await (child as unknown as { webContents: Electron.WebContents }).webContents.executeJavaScript("window.open('/child.html', '_blank')");
    }, String(remotePort));
    let popupId: string | undefined;
    let popupAttempts = 0;
    await expect.poll(async () => {
      const listed = await commandFromPane(remote, run, bridge, ["view", "list"], `remote-popup-${++popupAttempts}`);
      const views = (listed.answer.result as { views: { view_id: string; target: string }[] }).views;
      popupId = views.find((view) => view.target === `http://localhost:${remotePort}/child.html`)?.view_id;
      return Boolean(popupId);
    }, { timeout: 15_000 }).toBe(true);
    expect((await commandFromPane(remote, run, bridge, ["view", "close", popupId!], "remote-popup-close")).status).toBe(0);

    let decoyReads = 0;
    collisionServer = http.createServer((request, response) => {
      const body = request.url === "/collision.html"
        ? `<!doctype html><title>Remote collision</title><h1 id="origin">remote-device</h1><p id="asset">pending</p><p id="fetch">pending</p><p id="mapped">pending</p><p id="live">pending</p><script src="http://localhost:${(collisionServer!.address() as AddressInfo).port}/asset.js"></script><script>fetch('http://localhost:${(collisionServer!.address() as AddressInfo).port}/data').then(r => r.text()).then(t => document.getElementById('fetch').textContent = t); fetch('http://[::ffff:127.0.0.1]:${(collisionServer!.address() as AddressInfo).port}/data').then(r => r.text()).then(t => document.getElementById('mapped').textContent = t); new WebSocket('ws://localhost:${(collisionServer!.address() as AddressInfo).port}/live').onmessage = e => document.getElementById('live').textContent = e.data</script>`
        : request.url === "/asset.js" ? "document.getElementById('asset').textContent = 'remote-script'"
        : request.url === "/data" ? "remote-fetch" : null;
      response.writeHead(body ? 200 : 404, { "content-type": request.url === "/asset.js" ? "text/javascript" : "text/html", "access-control-allow-origin": "*" });
      response.end(body ?? "missing");
    });
    collisionServer.on("upgrade", (request, socket) => {
      const key = request.headers["sec-websocket-key"];
      if (request.url !== "/live" || typeof key !== "string") return socket.destroy();
      upgraded.add(socket);
      socket.on("close", () => upgraded.delete(socket));
      const accept = createHash("sha1").update(`${key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`).digest("base64");
      socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
      const payload = Buffer.from("remote-ws");
      socket.write(Buffer.concat([Buffer.from([0x81, payload.length]), payload]));
    });
    await new Promise<void>((resolve) => collisionServer!.listen(0, "::1", resolve));
    const collisionPort = (collisionServer.address() as AddressInfo).port;
    decoyServer = http.createServer((_request, response) => { decoyReads++; response.end("mac-decoy"); });
    decoyServer.on("upgrade", (_request, socket) => { decoyReads++; socket.destroy(); });
    await new Promise<void>((resolve) => decoyServer!.listen(collisionPort, "127.0.0.1", resolve));
    const collision = await commandFromPane(remote, run, bridge, ["browser", "open", `http://[::1]:${collisionPort}/collision.html`], "remote-collision");
    expect(collision.status, JSON.stringify(collision.answer)).toBe(0);
    await expect.poll(async () => app!.evaluate(async ({ BrowserWindow }) => {
      const child = BrowserWindow.getAllWindows()[0]?.contentView.children.find((entry) =>
        (entry as { webContents?: Electron.WebContents }).webContents?.getTitle() === "Remote collision") as { webContents: Electron.WebContents } | undefined;
      return child?.webContents.executeJavaScript("({ origin: document.getElementById('origin')?.textContent, asset: document.getElementById('asset')?.textContent, fetch: document.getElementById('fetch')?.textContent, mapped: document.getElementById('mapped')?.textContent, live: document.getElementById('live')?.textContent })");
    }), { timeout: 30_000 }).toEqual({ origin: "remote-device", asset: "remote-script", fetch: "remote-fetch", mapped: "remote-fetch", live: "remote-ws" });
    expect(decoyReads).toBe(0);
    expect((await commandFromPane(remote, run, bridge, ["view", "close", (collision.answer.result as { view_id: string }).view_id], "remote-collision-close")).status).toBe(0);

    const checkout = path.join(remote.root, "fixture");
    const html = path.join(checkout, "remote-preview.html");
    fs.writeFileSync(html, '<!doctype html><title>Remote HTML</title><link rel="stylesheet" href="style.css"><h1 id="proof">Remote HTML</h1><img id="pixel" src="pixel.png"><script src="preview.js"></script>');
    fs.writeFileSync(path.join(checkout, "style.css"), "#proof { color: rgb(12, 34, 56); }");
    fs.writeFileSync(path.join(checkout, "preview.js"), "document.getElementById('proof').dataset.script = 'remote-script'");
    for (const name of [".env", "secret.txt", "secret.json", "secret.js"]) fs.writeFileSync(path.join(checkout, name), "private");
    fs.writeFileSync(path.join(checkout, "pixel.png"), Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jR1sAAAAASUVORK5CYII=", "base64"));
    const outside = path.join(remote.root, "outside-secret.txt");
    fs.writeFileSync(outside, "outside");
    fs.symlinkSync(outside, path.join(checkout, "escape.txt"));
    const openedHtml = await commandFromPane(remote, run, bridge, ["browser", "open", html], "remote-html");
    expect(openedHtml.status, JSON.stringify(openedHtml.answer)).toBe(0);
    const preview = async () => app!.evaluate(async ({ BrowserWindow }) => {
      const child = BrowserWindow.getAllWindows()[0]?.contentView.children.find((entry) =>
        (entry as { webContents?: Electron.WebContents }).webContents?.getTitle() === "Remote HTML");
      if (!child) return null;
      const contents = (child as unknown as { webContents: Electron.WebContents }).webContents;
      return { url: contents.getURL(), state: await contents.executeJavaScript("({ script: document.getElementById('proof')?.dataset.script, color: getComputedStyle(document.getElementById('proof')).color, image: document.getElementById('pixel')?.naturalWidth })") as { script: string; color: string; image: number } };
    });
    await expect.poll(async () => (await preview())?.state, { timeout: 30_000 }).toEqual({ script: "remote-script", color: "rgb(12, 34, 56)", image: 1 });
    const previewUrl = (await preview())!.url;
    for (const name of [".env", ".git/config", "secret.txt", "secret.json", "secret.js"]) {
      expect((await fetch(new URL(name, previewUrl))).status, name).toBe(403);
    }
    let egressReads = 0;
    egressServer = http.createServer((_request, response) => { egressReads++; response.end("received"); });
    await new Promise<void>((resolve) => egressServer!.listen(0, "127.0.0.1", resolve));
    await app.evaluate(async ({ BrowserWindow }, port) => {
      const child = BrowserWindow.getAllWindows()[0]?.contentView.children.find((entry) =>
        (entry as { webContents?: Electron.WebContents }).webContents?.getTitle() === "Remote HTML") as { webContents: Electron.WebContents } | undefined;
      if (!child) throw new Error("Remote HTML page is missing for boundary check");
      await child.webContents.executeJavaScript(`Promise.allSettled([fetch('secret.txt'), fetch('http://127.0.0.1:${port}/leak'), new Promise(resolve => { const image = new Image(); image.onload = resolve; image.onerror = resolve; image.src = 'http://127.0.0.1:${port}/pixel'; })])`);
    }, (egressServer.address() as AddressInfo).port);
    expect(egressReads).toBe(0);
    const captureDir = process.env.HIDE_E2E_SCREENSHOT_DIR;
    if (captureDir) {
      fs.mkdirSync(captureDir, { recursive: true });
      const capture = await app.evaluate(async ({ BrowserWindow }) => {
        const windows = BrowserWindow.getAllWindows();
        if (windows.length !== 1) throw new Error(`Expected one candidate window, found ${windows.length}`);
        const child = windows[0]!.contentView.children.find((entry) =>
          (entry as { webContents?: Electron.WebContents }).webContents?.getTitle() === "Remote HTML");
        if (!child) throw new Error("Remote native page is missing");
        return (await (child as unknown as { webContents: Electron.WebContents }).webContents.capturePage()).toPNG().toString("base64");
      });
      fs.writeFileSync(path.join(captureDir, "remote-html-native.png"), Buffer.from(capture, "base64"));
    }
    const escape = new URL("escape.txt", previewUrl);
    expect((await fetch(escape)).status).toBe(403);
    const parent = new URL(previewUrl);
    const traversal = await new Promise<number>((resolve, reject) => {
      http.get({ hostname: parent.hostname, port: parent.port, path: `${parent.pathname.slice(0, parent.pathname.lastIndexOf("/") + 1)}%2e%2e/outside-secret.txt` }, (response) => { response.resume(); resolve(response.statusCode ?? 0); }).on("error", reject);
    });
    expect(traversal).toBe(403);
    const htmlId = (openedHtml.answer.result as { view_id: string }).view_id;
    const devId = (openedDev.answer.result as { view_id: string }).view_id;
    const closingNativeIds = await app.evaluate(async ({ BrowserWindow }, urls) =>
      BrowserWindow.getAllWindows()[0]?.contentView.children.flatMap((entry) => {
        const contents = (entry as { webContents?: Electron.WebContents }).webContents;
        return contents && urls.includes(contents.getURL()) ? [contents.id] : [];
      }) ?? [], [previewUrl, routed]);
    expect(closingNativeIds).toHaveLength(2);
    await showDevice(page, state, nodeOf(run.env));
    if (process.env.HIDE_E2E_SSH_PID && process.env.HIDE_E2E_SSH_CONFIG) {
      const sshPid = Number(process.env.HIDE_E2E_SSH_PID);
      const sshCommand = spawnSync("ps", ["-p", String(sshPid), "-o", "command="], { encoding: "utf8" });
      expect(sshCommand.status).toBe(0);
      expect(sshCommand.stdout).toContain(process.env.HIDE_E2E_SSH_CONFIG);
      expect(sshCommand.stdout).toContain("[listener]");
      const routeBody = (id: string, load: number) => JSON.stringify({
        device_id: "ssh-e2e", checkout_path: fs.realpathSync(path.join(remote.root, "fixture")),
        id, load, owner_pid: app!.process().pid,
      });
      const active = await commandFromPane(remote, run, bridge, ["browser", "open", `http://localhost:${remotePort}/active.html`], "remote-active-open");
      expect(active.status, JSON.stringify(active.answer)).toBe(0);
      const activeView = active.answer.result as { view_id: string; load: number };
      const activeResponse = await fetch(`http://127.0.0.1:${state.port}/browser-route`, {
        method: "POST", headers: { Authorization: `Bearer ${state.token}`, "Content-Type": "application/json" },
        body: routeBody(activeView.view_id, activeView.load),
      });
      expect(activeResponse.status).toBe(200);
      const activeRoute = await activeResponse.json() as { url: string };
      const stalled = await commandFromPane(remote, run, bridge, ["browser", "open", `http://localhost:${remotePort}/stall.html`], "remote-stalled-open");
      expect(stalled.status, JSON.stringify(stalled.answer)).toBe(0);
      const stalledView = stalled.answer.result as { view_id: string; load: number };
      process.kill(sshPid, "SIGSTOP");
      let pending: Promise<Response> | undefined;
      try {
        pending = fetch(`http://127.0.0.1:${state.port}/browser-route`, {
          method: "POST", headers: { Authorization: `Bearer ${state.token}`, "Content-Type": "application/json" },
          body: routeBody(stalledView.view_id, stalledView.load),
        });
        expect(await Promise.race([pending.then(() => "completed"), new Promise((resolve) => setTimeout(() => resolve("pending"), 250))])).toBe("pending");
        const released = await fetch(`http://127.0.0.1:${state.port}/browser-route`, {
          method: "DELETE", headers: { Authorization: `Bearer ${state.token}`, "Content-Type": "application/json" },
          body: routeBody(activeView.view_id, activeView.load),
          signal: AbortSignal.timeout(2_500),
        });
        expect(released.status).toBe(204);
        await expect.poll(() => canBindLoopback(Number(new URL(activeRoute.url).port), "127.0.0.1"), { timeout: 2_500 }).toBe(true);
        const discardCount = () => fs.readFileSync(daemonLog, "utf8").split("\n").filter((line) => line.includes('"kind":"route.discarded"')).length;
        const beforeCancel = discardCount();
        for (let attempt = 0; attempt < 6; attempt++) {
          const waitingRoute = attempt === 0 ? pending! : fetch(`http://127.0.0.1:${state.port}/browser-route`, {
            method: "POST", headers: { Authorization: `Bearer ${state.token}`, "Content-Type": "application/json" },
            body: routeBody(stalledView.view_id, stalledView.load),
          });
          if (attempt > 0) {
            expect(await Promise.race([waitingRoute.then(() => "completed"), new Promise((resolve) => setTimeout(() => resolve("pending"), 100))])).toBe("pending");
          }
          const canceled = await fetch(`http://127.0.0.1:${state.port}/browser-route`, {
            method: "DELETE", headers: { Authorization: `Bearer ${state.token}`, "Content-Type": "application/json" },
            body: routeBody(stalledView.view_id, stalledView.load), signal: AbortSignal.timeout(2_500),
          });
          expect(canceled.status).toBe(204);
          const response = await waitingRoute;
          expect(response.status).toBe(409);
          expect(await response.json()).toMatchObject({ reason: "view_unavailable" });
          await expect.poll(discardCount, { timeout: 2_500 }).toBe(beforeCancel + attempt + 1);
        }
      } finally {
        process.kill(sshPid, "SIGCONT");
      }
      expect((await commandFromPane(remote, run, bridge, ["view", "close", activeView.view_id], "remote-active-close")).status).toBe(0);
      expect((await commandFromPane(remote, run, bridge, ["view", "close", stalledView.view_id], "remote-stalled-close")).status).toBe(0);
    }
    expect((await commandFromPane(remote, run, bridge, ["view", "close", htmlId], "remote-html-close")).status).toBe(0);
    expect((await commandFromPane(remote, run, bridge, ["view", "close", devId], "remote-dev-close")).status).toBe(0);
    await expect.poll(async () => app!.evaluate(async ({ BrowserWindow }, ids) =>
      BrowserWindow.getAllWindows()[0]?.contentView.children.some((entry) =>
        ids.includes((entry as { webContents?: Electron.WebContents }).webContents?.id ?? -1)) ?? false, closingNativeIds
    ), { timeout: 15_000 }).toBe(false);
    for (const url of [previewUrl, routed]) {
      await expect.poll(async () => {
        try { await fetch(url, { signal: AbortSignal.timeout(500) }); return false; }
        catch { return true; }
      }, { timeout: 10_000 }).toBe(true);
    }
    expect((await commandFromPane(local, run, null, ["view", "close", (localBrowser.answer.result as { view_id: string }).view_id], "local-dev-close")).status).toBe(0);
    await expect.poll(async () => app!.evaluate(async ({ BrowserWindow }) =>
      BrowserWindow.getAllWindows()[0]?.contentView.children.filter((entry) =>
        ["Remote dev", "Remote HTML"].includes((entry as { webContents?: Electron.WebContents }).webContents?.getTitle() ?? "")).length ?? 0
    ), { timeout: 15_000 }).toBe(0);
    const upperAddress = `FILE://${fs.realpathSync(html)}`;
    await showDevice(page, state, "ssh-e2e");
    const upper = await commandFromPane(remote, run, bridge, ["browser", "open", upperAddress], "remote-uppercase-file");
    expect(upper.status, JSON.stringify(upper.answer)).toBe(0);
    const upperViews = await commandFromPane(remote, run, bridge, ["view", "list"], "remote-uppercase-list");
    expect((upperViews.answer.result as { views: { target: string }[] }).views.some((view) => view.target === upperAddress)).toBe(true);
    await expect.poll(async () => app!.evaluate(async ({ BrowserWindow }) => {
      const child = BrowserWindow.getAllWindows()[0]?.contentView.children.find((entry) =>
        (entry as { webContents?: Electron.WebContents }).webContents?.getTitle() === "Remote HTML") as { webContents: Electron.WebContents } | undefined;
      return child?.webContents.executeJavaScript("document.cookie") as Promise<string> | undefined;
    }), { timeout: 20_000 }).toBe("");
    await app.evaluate(async ({ BrowserWindow }) => {
      const child = BrowserWindow.getAllWindows()[0]?.contentView.children.find((entry) =>
        (entry as { webContents?: Electron.WebContents }).webContents?.getTitle() === "Remote HTML") as { webContents: Electron.WebContents } | undefined;
      if (!child) throw new Error("Uppercase FILE page is missing");
      await child.webContents.executeJavaScript("window.open('https://example.test/leak', '_blank'); location.href = 'https://example.test/leak'");
    });
    await new Promise((resolve) => setTimeout(resolve, 250));
    const afterFilePopup = await commandFromPane(remote, run, bridge, ["view", "list"], "remote-uppercase-popup-check");
    expect(afterFilePopup.status).toBe(0);
    expect((afterFilePopup.answer.result as { views: { target: string }[] }).views.some((view) => view.target.includes("example.test"))).toBe(false);
    expect((await commandFromPane(remote, run, bridge, ["view", "close", (upper.answer.result as { view_id: string }).view_id], "remote-uppercase-close")).status).toBe(0);
    const mapped = await commandFromPane(remote, run, bridge, ["browser", "open", `http://[::ffff:127.0.0.1]:${remotePort}/remote.html`], "remote-mapped-loopback");
    expect(mapped.status, JSON.stringify(mapped.answer)).toBe(0);
    await expect.poll(async () => app!.evaluate(async ({ BrowserWindow }) => {
      const child = BrowserWindow.getAllWindows()[0]?.contentView.children.find((entry) =>
        (entry as { webContents?: Electron.WebContents }).webContents?.getTitle() === "Remote dev") as { webContents: Electron.WebContents } | undefined;
      return child?.webContents.getURL() ?? null;
    }), { timeout: 20_000 }).not.toBeNull();
    const mappedRoute = await app.evaluate(async ({ BrowserWindow }) => {
      const child = BrowserWindow.getAllWindows()[0]?.contentView.children.find((entry) =>
        (entry as { webContents?: Electron.WebContents }).webContents?.getTitle() === "Remote dev") as { webContents: Electron.WebContents } | undefined;
      return child?.webContents.getURL() ?? "";
    });
    expect(new URL(mappedRoute).hostname).toBe("127.0.0.1");
    expect(new URL(mappedRoute).port).not.toBe(String(remotePort));
    expect((await commandFromPane(remote, run, bridge, ["view", "close", (mapped.answer.result as { view_id: string }).view_id], "remote-mapped-close")).status).toBe(0);
    await showDevice(page, state, nodeOf(run.env));
    // A device pane's open stays behind until the operator is on the device.
    const behind = await commandFromPane(remote, run, bridge, ["browser", "open", `http://localhost:${remotePort}/remote.html`], "remote-behind");
    expect(behind.status, JSON.stringify(behind.answer)).toBe(0);
    expect(await page.locator("[data-workspace-screen]").getAttribute("data-workspace-screen")).not.toContain("ssh-e2e");
    await showDevice(page, state, "ssh-e2e");
    const front = await commandFromPane(remote, run, bridge, ["browser", "open", `http://localhost:${remotePort}/remote.html`], "remote-front");
    expect(front.status, JSON.stringify(front.answer)).toBe(0);
    await expect.poll(async () => (await native())?.live, { timeout: 20_000 }).toBe("remote-live");
    const ownedUrl = (await native())!.url;
    const candidatePid = app.process().pid;
    expect(candidatePid).toBeGreaterThan(0);
    app.process().kill("SIGKILL");
    await expect.poll(async () => {
      try { await fetch(ownedUrl, { signal: AbortSignal.timeout(500) }); return false; }
      catch { return true; }
    }, { timeout: 10_000 }).toBe(true);
    const noRenderer = await hookFromPane(remote, path.dirname(bridge), "codex", "remote-no-renderer-hook");
    expect(noRenderer.status).toBe(0);
    expect(noRenderer.context).not.toContain("Hide Workspace control is available");
  } catch (error) {
    console.log(hostLog(run.env).map((line) => JSON.stringify(line)).join("\n"));
    console.log(fs.readFileSync(daemonLog, "utf8"));
    throw error;
  } finally {
    if (tlsPartition) await app?.evaluate(({ session }, partition) => session.fromPartition(partition).setCertificateVerifyProc(null), tlsPartition)
      .catch((error: unknown) => console.error("Private TLS verifier cleanup failed", error));
    await app?.close().catch(() => undefined);
    for (const socket of upgraded) socket.destroy();
    await new Promise<void>((resolve) => devServer?.close(() => resolve()) ?? resolve());
    await new Promise<void>((resolve) => tlsServer?.close(() => resolve()) ?? resolve());
    await new Promise<void>((resolve) => collisionServer?.close(() => resolve()) ?? resolve());
    await new Promise<void>((resolve) => decoyServer?.close(() => resolve()) ?? resolve());
    await new Promise<void>((resolve) => egressServer?.close(() => resolve()) ?? resolve());
    await new Promise<void>((resolve) => twinServer?.close(() => resolve()) ?? resolve());
    run.cleanup();
    await endChild(daemon).finally(() => { remote.stop(); second?.stop(); local.stop(); });
  }
});
