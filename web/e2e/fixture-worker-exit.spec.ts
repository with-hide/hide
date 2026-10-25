// @platform: Node-managed worker exit must end pending native CLI trees
// before their shared server/root, including the Windows launcher children.
import { expect, test } from "@playwright/test";
import { fork, spawnSync, type ChildProcess } from "node:child_process";
import { createRequire } from "node:module";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { herdrBinary } from "./herdr-fixture";
import { endWindowsProcesses, fixtureProgram, type WindowsProcess } from "./platform-fixture";
import { fixtureShimPath } from "./shims/build";
import { afterCleanup } from "./worker-owned";

// Use the desktop host's existing bundler to run the actual fixture modules
// in an independent Node owner, without starting Electron or building hided.
const { buildSync } = createRequire(new URL("../../desktop/package.json", import.meta.url))("esbuild");
type Start = { leader: number; owned: WindowsProcess[] };
type Private = { root: string; env: NodeJS.ProcessEnv };

function pendingWorker(root: string, refuse: boolean): { child: ChildProcess; stderr: () => string } {
  const binary = herdrBinary();
  const processTool = process.platform === "win32" ? fixtureShimPath("hide-processes") : "";
  const proxy = fixtureProgram(root, "pending-herdr", `
const fs = require("node:fs"), path = require("node:path");
const { spawn, spawnSync, execFileSync } = require("node:child_process");
const root = ${JSON.stringify(root)}, binary = ${JSON.stringify(binary)}, tool = ${JSON.stringify(processTool)};
const args = process.argv.slice(2);
function tree(pid) {
  return execFileSync(tool, ["tree", String(pid)], { encoding: "utf8", timeout: 10000 }).trim().split(/\\r?\\n/).filter(Boolean).map(line => {
    const [id, created] = line.split(" "); return { id: Number(id), created };
  });
}
function alive(start) {
  if (tool) return start.owned.filter(item => tree(item.id).some(now => now.id === item.id && now.created === item.created)).map(item => item.id);
  const ps = spawnSync("/bin/ps", ["-A", "-o", "pid=,pgid=,stat="], { encoding: "utf8", timeout: 10000 });
  if (ps.error || ps.status !== 0) throw new Error("process observation failed");
  return ps.stdout.trim().split(/\\r?\\n/).flatMap(line => {
    const [pid, group, state] = line.trim().split(/\\s+/);
    return Number(group) === start.leader && !state.startsWith("Z") ? [Number(pid)] : [];
  });
}
if (args[0] === "server" && args.length === 1) {
  const keys = ["HERDR_SOCKET_PATH", "HERDR_CONFIG_PATH", "HERDR_SESSION", "HOME"];
  fs.writeFileSync(path.join(root, "private.json"), JSON.stringify({ root: process.env.HIDE_E2E_ROOT, env: Object.fromEntries(keys.map(key => [key, process.env[key]])) }));
}
if (args[0] === "server" && args[1] === "stop") {
  const live = ["one", "two"].flatMap(name => {
    const file = path.join(root, name + ".pending");
    return fs.existsSync(file) ? alive(JSON.parse(fs.readFileSync(file, "utf8"))) : [];
  });
  fs.writeFileSync(path.join(root, "server-stop.json"), JSON.stringify(live));
}
const cli = spawn(binary, args, { env: process.env, stdio: ["ignore", "pipe", "pipe"] });
cli.on("error", error => { console.error(error); process.exit(1); });
cli.stdout.pipe(process.stdout); cli.stderr.pipe(process.stderr);
if (args[0] === "agent" && args[1] === "start") {
  const leader = tool ? process.ppid : process.pid;
  fs.writeFileSync(path.join(root, args[2] + ".pending"), JSON.stringify({ leader, owned: tool ? tree(leader) : [] }));
  // Even after the real CLI answers, this real client wrapper stays pending
  // until its owning fixture ends it. No Herdr response is substituted.
  setInterval(() => {}, 1000);
} else cli.on("exit", code => process.exit(code === null ? 1 : code));
`);
  const owner = path.join(root, "owner.cjs");
  buildSync({
    stdin: { contents: `
import { startHerdr } from ${JSON.stringify(fileURLToPath(new URL("./herdr-fixture.ts", import.meta.url)))};
const cp = require("node:child_process");
if (${refuse}) {
  const kill = process.kill.bind(process);
  process.kill = (pid, signal) => { if (pid < 0) throw new Error("injected CLI termination refusal"); return kill(pid, signal); };
  const exec = cp.execFileSync;
  cp.execFileSync = (bin, args, options) => {
    if (bin === ${JSON.stringify(processTool)} && args[0] === "end" && args[1] === "-") throw new Error("injected CLI termination refusal");
    return exec(bin, args, options);
  };
}
process.on("message", () => process.exit(37));
process.on("disconnect", () => process.exit(37));
startHerdr().catch(error => { console.error(error); process.exit(1); });
`, resolveDir: fileURLToPath(new URL("../..", import.meta.url)) },
    bundle: true, platform: "node", format: "cjs", outfile: owner,
  });
  const child = fork(owner, [], { env: { ...process.env, HIDE_E2E_HERDR_BIN: proxy }, stdio: ["ignore", "ignore", "pipe", "ipc"] });
  let stderr = "";
  child.stderr!.on("data", chunk => { stderr += String(chunk); });
  return { child, stderr: () => stderr };
}

async function withPendingWorker(refuse: boolean, check: (root: string, fixture: Private, stderr: string) => void): Promise<void> {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "hide-worker-exit-"));
  let child: ChildProcess | undefined;
  let fixture: Private | undefined;
  const starts: Start[] = [];
  let failure: unknown;
  try {
    const worker = pendingWorker(root, refuse);
    child = worker.child;
    await expect.poll(() => ["one", "two"].every(name => fs.existsSync(path.join(root, name + ".pending"))), { timeout: 10_000 }).toBe(true);
    fixture = JSON.parse(fs.readFileSync(path.join(root, "private.json"), "utf8")) as Private;
    for (const name of ["one", "two"]) starts.push(JSON.parse(fs.readFileSync(path.join(root, name + ".pending"), "utf8")) as Start);
    expect(child.exitCode).toBeNull();
    const exited = new Promise<number | null>((resolve) => child!.once("close", resolve));
    child.send("exit");
    expect(await exited).toBe(37);
    check(root, fixture, worker.stderr());
  } catch (error) { failure = error; }
  try {
    if (child && child.exitCode === null && child.signalCode === null) {
      const closed = new Promise<void>((resolve) => child!.once("close", () => resolve()));
      child.kill("SIGKILL");
      await closed;
    }
    for (const name of ["one", "two"]) {
      const file = path.join(root, name + ".pending");
      if (fs.existsSync(file)) {
        const start = JSON.parse(fs.readFileSync(file, "utf8")) as Start;
        if (!starts.some((item) => item.leader === start.leader)) starts.push(start);
      }
    }
    // Failure injection deliberately leaves owned clients alive. End only the
    // recorded identities/groups, then the same private server, before removal.
    for (const start of starts) {
      if (process.platform === "win32") endWindowsProcesses(start.owned);
      else {
        const leader = spawnSync("/bin/ps", ["-p", String(start.leader), "-o", "args="], { encoding: "utf8", timeout: 5_000 });
        if (leader.stdout.includes(path.join(root, "pending-herdr"))) process.kill(-start.leader, "SIGKILL");
      }
    }
    if (!fixture && fs.existsSync(path.join(root, "private.json"))) fixture = JSON.parse(fs.readFileSync(path.join(root, "private.json"), "utf8")) as Private;
    if (fixture) {
      spawnSync(herdrBinary(), ["server", "stop"], { env: { ...process.env, ...fixture.env }, timeout: 10_000 });
      if (process.platform === "win32") endWindowsProcesses([], fixture.root);
      fs.rmSync(fixture.root, { recursive: true, force: true });
    }
    fs.rmSync(root, { recursive: true, force: true });
  } catch (cleanup) {
    if (failure !== undefined) throw afterCleanup(failure, () => { throw cleanup; });
    throw cleanup;
  }
  if (failure !== undefined) throw failure;
}

test("worker exit ends pending CLI trees before server and root cleanup", { tag: "@platform" }, async () => {
  await withPendingWorker(false, (root, fixture, stderr) => {
    expect(JSON.parse(fs.readFileSync(path.join(root, "server-stop.json"), "utf8"))).toEqual([]);
    expect(fs.existsSync(fixture.root)).toBe(false);
    expect(stderr).not.toContain("e2e fixture stop failed");
  });
});

test("worker exit preserves the root and reports unconfirmed CLI termination", { tag: "@platform" }, async () => {
  await withPendingWorker(true, (root, fixture, stderr) => {
    expect(JSON.parse(fs.readFileSync(path.join(root, "server-stop.json"), "utf8")).length).toBeGreaterThan(0);
    expect(fs.existsSync(fixture.root)).toBe(true);
    expect(stderr).toContain(`fixture CLI exit unconfirmed; preserve ${fixture.root}`);
    expect(stderr).toContain("injected CLI termination refusal");
  });
});
