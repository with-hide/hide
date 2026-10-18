// An isolated `hided` on a private HOME and state directory, for the web e2e
// lanes. It reads the daemon's state file for the loopback origin and token,
// so nothing here touches the operator's running daemon.

import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { linkFixtureTranscripts, type HerdrFixture } from "./herdr-fixture";
import { ownUntilWorkerExit } from "./worker-owned";
import { bundledExecutable, linkBundle } from "./bundled-app";
import { endWindowsProcesses, fixtureExecutable, fixtureHomeEnv, fixtureLoginShellPath, fixtureOpenCommand, inheritedFixtureEnv, windowsProcessTree, type WindowsProcess } from "./platform-fixture";

/**
 * `restart` stops the daemon and starts it again on the same state directory
 * and port, as a daemon restart does: the core loses what it held in memory,
 * the host keeps its id, and the browser keeps its origin and so its drafts.
 */
export type Daemon = {
  /** Owned fixture process, used for isolated resource measurements. */
  pid: number;
  origin: string;
  token: string;
  home: string;
  stateDir: string;
  hostId: string;
  /** The core's own node id, as the state folder's `node.json` records it. */
  node: string;
  stop: () => void;
  /** `beforeStart` runs on the daemon's state directory while it is down, and may wait. */
  restart: (beforeStart?: (stateDir: string) => void | Promise<void>) => Promise<Daemon>;
};

/**
 * Starts a fixture daemon tied to this worker. On Unix it takes the channel on
 * descriptor 3 as its owner (`OwnerWatch`), so it ends, with what it started,
 * when the worker ends however it ends: a SIGKILL or an OOM kill runs no exit
 * callback. Windows passes an owner as a job, which Node cannot make.
 */
export function spawnDaemon(binary: string, env: NodeJS.ProcessEnv): ChildProcess {
  if (env.HOME && env.PATH) fixtureLoginShellPath(env.HOME, env.PATH);
  if (process.platform === "win32") return spawn(binary, [], { env, stdio: ["ignore", "pipe", "pipe"] });
  return spawn(binary, [], { env: { ...env, HIDE_PROCESS_OWNER_FD: "3" }, stdio: ["ignore", "pipe", "pipe", "pipe"] });
}

/**
 * Ends a fixture daemon on Unix through `hide stop`, which sends SIGTERM,
 * gives the graceful stop five seconds, then ends the daemon's tree and fails
 * if it still runs. A stopped daemon holds that SIGTERM pending, so it is
 * continued first. True once the end is confirmed; a daemon that never wrote
 * its state has nothing to save and is killed, and false says its exit is not
 * observed yet. Throws, naming `root` to keep, when the stop is not confirmed.
 */
export function stopDaemon(child: ChildProcess, binary: string, env: NodeJS.ProcessEnv, root: string): boolean {
  if (child.pid === undefined || child.exitCode !== null || child.signalCode !== null) return true;
  child.kill("SIGCONT");
  if (!fs.existsSync(path.join(env.HIDE_STATE_DIR!, "hided.json"))) {
    child.kill("SIGKILL");
    return false;
  }
  const cli = path.join(path.dirname(binary), fixtureExecutable("hide"));
  const stopped = spawnSync(cli, ["stop"], { env, encoding: "utf8", timeout: 20_000 });
  if (stopped.error || stopped.status !== 0) {
    throw new Error(`hided ${child.pid} did not stop; preserve ${root}: ${stopped.error?.message ?? (stopped.stderr || stopped.stdout || `exit ${stopped.status}`)}`);
  }
  return true;
}

/**
 * `extraEnv` is laid over the daemon's environment; an undefined value leaves that variable unset.
 * `bundled` runs the daemon from the worker's app bundle of the debug binaries (`bundled-app.ts`),
 * the one place a daemon installs the kit (`hide_kit::bundled_kit_dir`); the kit then writes into
 * the fixture's private HOME and nowhere else.
 * `seedHideAi: false` leaves Hide AI unchosen, as a Mac that has never been asked is, for a spec
 * about the first-run rule.
 * `bundledHerdr` runs a bundled daemon from the bundle that also carries the fixture's pinned Herdr, so the
 * kit installs and removes agents' Herdr integrations with it, as a packaged app does.
 */
export async function startHided(herdr: HerdrFixture, label = "s2", homeOverride?: string, extraEnv: NodeJS.ProcessEnv = {}, bundled = false, options: { seedHideAi?: boolean; bundledHerdr?: boolean } = {}): Promise<Daemon> {
  // The daemon places pane-bootstrap.sock below this directory. Keep the
  // fixture root short enough for macOS's Unix socket path limit.
  const dir = fs.realpathSync.native(fs.mkdtempSync(path.join(os.tmpdir(), "hde-")));
  const home = homeOverride ?? path.join(dir, "home");
  fs.mkdirSync(path.join(home, "projects", "alpha"), { recursive: true });
  fs.mkdirSync(path.join(home, "projects", ".hidden"), { recursive: true });
  fs.writeFileSync(path.join(home, "projects", "notes.txt"), "x");
  linkFixtureTranscripts(herdr, home);
  // Hide AI asks no model until an agent is chosen (PRD settings-cleanup B47), and a fixture
  // Mac has no signed-in agent for the first-run rule to pick, so the operator here has
  // already chosen the `claude` fixture shim; a home that brought its own choice keeps it.
  const aiFile = aiSettingsFile(home);
  if (options.seedHideAi !== false && !fs.existsSync(aiFile)) {
    fs.mkdirSync(path.dirname(aiFile), { recursive: true });
    fs.writeFileSync(aiFile, JSON.stringify({ provider: "claude" }));
  }
  const bundle = options.bundledHerdr ? "herdr" : "plain";
  if (bundled) linkBundle(bundle, options.bundledHerdr ? herdr.bin : undefined);
  return launch(herdr, label, dir, home, "0", extraEnv, bundled ? bundledExecutable("hided", bundle) : undefined);
}

/** Where the daemon keeps Hide AI's choice under a home: the platform's state folder, `hide/ai.json`. */
export function aiSettingsFile(home: string): string {
  const stateUnderHome = process.platform === "darwin" ? path.join("Library", "Application Support") : process.platform === "win32" ? path.join("AppData", "Local") : path.join(".local", "state");
  return path.join(home, stateUnderHome, "hide", "ai.json");
}

/** 100 ms ticks a started daemon is given to write its state file and answer /health: 30 s, three times the product's own bound. */
const START_WAIT_TICKS = 300;

async function launch(herdr: HerdrFixture, label: string, dir: string, home: string, port: string, extraEnv: NodeJS.ProcessEnv, bundledAt?: string): Promise<Daemon> {
  const env = inheritedFixtureEnv();
  const statePath = path.join(dir, "hide", "hided.json");
  fs.rmSync(statePath, { force: true });
  const binary = bundledAt ?? path.resolve("..", "target", "debug", fixtureExecutable("hided"));
  const daemonEnv: NodeJS.ProcessEnv = {
    ...env,
    ...fixtureHomeEnv(home),
    HIDE_STATE_DIR: path.join(dir, "hide"),
    HIDE_KEEP_ALIVE: "1",
    HIDE_PORT: port,
    HIDED_UI_DIR: path.resolve("dist"),
    HERDR_SOCKET_PATH: herdr.socket,
    HERDR_BIN_PATH: herdr.bin,
    // Catalog discovery probes every provider on hided's PATH, so use the
    // same controlled shim and system tools as the private Herdr server.
    // Specs adding a shim prepend it to this path through extraEnv.
    PATH: herdr.fixturePath,
    // `open_external` must not launch a GUI application on the runner.
    HIDE_OPEN_COMMAND: fixtureOpenCommand(herdr.root),
    ...extraEnv,
  };
  const child = spawnDaemon(binary, daemonEnv);
  // A daemon that cannot be spawned (no debug build in this worktree) is an
  // error event; unheard, it kills the worker before any test cleanup runs.
  let spawnFailed = null as Error | null;
  child.once("error", (error) => {
    spawnFailed = error;
  });
  const logDir = process.env.HIDE_E2E_SCREENSHOT_DIR;
  if (logDir) {
    const log = fs.createWriteStream(path.join(logDir, `hided-${label}-${path.basename(dir)}.log`), { flags: "a" });
    // Two pipes share the file: the first stream to end must not close it
    // under the other's last write ("write after end" on a restart), so it
    // closes once the daemon's output has ended on both.
    child.stdout?.pipe(log, { end: false });
    child.stderr?.pipe(log, { end: false });
    child.once("close", () => log.end());
  }
  const exited = new Promise<void>((resolve) => child.once("exit", () => resolve()));
  // Panes Herdr started under the home keep their working folders locked on
  // Windows until its server's processes are gone, so the daemon's directory
  // (the home is inside it) is removed by Herdr's stop, after them.
  const removeDir = () => {
    // The daemon may still be writing its state file (and, with S3, staged
    // files) as it dies; removing the directory under it fails with ENOTEMPTY.
    for (let attempt = 0; attempt < 40; attempt += 1) {
      try {
        fs.rmSync(dir, { recursive: true, force: true });
        return;
      } catch {
        // Synchronous worker-exit cleanup cannot await a timer. Keep the
        // same 50 ms filesystem retry interval without a Unix child tool.
        Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 50);
      }
    }
    fs.rmSync(dir, { recursive: true, force: true });
  };
  // A killed daemon on Windows keeps the folders it holds open (the checkouts
  // it watches) until it is gone, and Herdr's stop deletes the fixture root
  // next. So its tree is listed while it runs, then ended, and this returns
  // once none of it is left.
  const end = () => {
    if (process.platform !== "win32") {
      stopDaemon(child, binary, daemonEnv, dir);
      return;
    }
    if (!child.pid || child.exitCode !== null || child.signalCode !== null) {
      child.kill();
      return;
    }
    let tree: WindowsProcess[] = [];
    let failure: unknown;
    try { tree = windowsProcessTree(child.pid); } catch (error) { failure = error; }
    child.kill();
    endWindowsProcesses(tree);
    if (failure !== undefined) throw failure;
  };
  // The directory goes only after the daemon's end is confirmed: one that
  // throws keeps it, named in the error.
  const { stop, disown } = ownUntilWorkerExit(() => {
    end();
    herdr.afterStop(removeDir);
  });
  // The product itself gives a started daemon `HEALTHY_WITHIN` (10 s, `hided/src/cli.rs`) to answer
  // /health, and a loaded Windows runner needs most of it, so this waits at least that long. A daemon
  // that has already exited is named at once instead of after the bound.
  let exitedWith = null as string | null;
  child.once("exit", (code, signal) => {
    exitedWith = signal ? `signal ${signal}` : `code ${code}`;
  });
  for (let i = 0; i < START_WAIT_TICKS; i += 1) {
    if (spawnFailed) {
      stop();
      throw new Error(`hided did not start from ${binary}: ${spawnFailed.message}; run cargo build -p hided in this worktree`);
    }
    if (exitedWith !== null && !fs.existsSync(statePath)) {
      stop();
      throw new Error(`hided exited with ${exitedWith} before it wrote a state file; its output is in hided-${label}-${path.basename(dir)}.log under HIDE_E2E_SCREENSHOT_DIR`);
    }
    if (fs.existsSync(statePath)) {
      try {
        // The file may be mid-write on the first read; the next tick reads it whole.
        const state = JSON.parse(fs.readFileSync(statePath, "utf8")) as { port: number; token: string };
        const origin = `http://127.0.0.1:${state.port}`;
        if ((await fetch(`${origin}/health`)).ok) {
          const hostId = fs.readFileSync(path.join(dir, "hide", "host-id"), "utf8").trim();
          const node = (JSON.parse(fs.readFileSync(path.join(dir, "hide", "node.json"), "utf8")) as { node: string }).node;
          const restart = async (beforeStart?: (stateDir: string) => void | Promise<void>) => {
            end();
            await exited;
            await beforeStart?.(path.join(dir, "hide"));
            // The next daemon takes over the directory and its removal.
            disown();
            return launch(herdr, label, dir, home, String(state.port), extraEnv, bundledAt);
          };
          return { pid: child.pid!, origin, token: state.token, home: fs.realpathSync(home), stateDir: path.join(dir, "hide"), hostId, node, stop, restart };
        }
      } catch {
        /* still starting */
      }
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  stop();
  const wrote = fs.existsSync(statePath);
  throw new Error(`hided ${wrote ? "wrote a state file but did not answer /health" : "did not write a state file"} within ${(START_WAIT_TICKS * 100) / 1000} s`);
}

/**
 * Gives the daemon's private HOME an ssh config with one Host, so Add device has an entry to
 * choose. The host name is reserved for documentation and never resolves, which keeps the
 * device registered and unreachable; hided reads this file and the operator's own is untouched.
 */
export function writeSshHost(daemon: Daemon, alias: string): void {
  const dir = path.join(daemon.home, ".ssh");
  fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  fs.writeFileSync(path.join(dir, "config"), `Host ${alias}\n  HostName ${alias}.invalid\n  User e2e\n`, { mode: 0o600 });
}
