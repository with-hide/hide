// Exercise the real fixture teardown and filesystem with Electron and service
// calls replaced at their external boundaries. No native process is started.
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { ChildProcess, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { isolate, launch, type Isolated } from "./fixture";
import { FUSED_ELECTRON_ENV } from "./fused-env";
import { endWindowsProcesses } from "../../web/e2e/platform-fixture";
import { copyFixtureShim } from "../../web/e2e/shims/build";

type Guard = (fixtures: Record<string, never>, use: () => Promise<void>, info: { tags: string[] }) => Promise<void>;
const boundary = vi.hoisted(() => ({ launch: vi.fn(), guard: null as Guard | null }));
vi.mock("@playwright/test", () => ({
  _electron: { launch: boundary.launch },
  test: { extend: (fixtures: { focusGuard: [Guard, unknown] }) => { boundary.guard = fixtures.focusGuard[0]; return {}; } },
  expect,
}));
vi.mock("node:child_process", async (original) => ({
  ...await original<typeof import("node:child_process")>(),
  spawnSync: vi.fn((command: string) => {
    throw new Error(`unexpected fixture process: ${command}`);
  }),
}));

// Ending the processes that run from a root is a Windows PowerShell call; this
// test starts none, so the boundary is the call itself: which processes
// (none owned beyond those under the folder) and which folder, while it exists.
vi.mock("../../web/e2e/platform-fixture", async (original) => ({
  ...await original<typeof import("../../web/e2e/platform-fixture")>(),
  endWindowsProcesses: vi.fn((owned: unknown[], folder: string) => {
    if (owned.length !== 0 || !fs.existsSync(folder)) throw new Error(`unexpected endWindowsProcesses(${JSON.stringify(owned)}, ${folder})`);
  }),
}));

/** The root is cleaned by ending what runs from it first on Windows, and by nothing else elsewhere. */
function expectRootEnded(run: Isolated): void {
  if (process.platform === "win32") expect(endWindowsProcesses).toHaveBeenCalledWith([], run.root);
  else expect(endWindowsProcesses).not.toHaveBeenCalled();
}

// The no-op opener is a finished program the e2e entry point built; here the
// copy is the boundary: which program, and where in the private root it lands.
vi.mock("../../web/e2e/shims/build", () => ({
  copyFixtureShim: vi.fn((name: string, executable: string) => {
    const privateRoot = path.dirname(path.dirname(executable));
    if (name !== "noop" || path.basename(executable) !== "hide-open.exe" || path.basename(path.dirname(executable)) !== "bin"
      || !path.basename(privateRoot).startsWith("hide-desktop-")) {
      throw new Error(`unexpected fixture program copy: ${name} -> ${executable}`);
    }
    fs.writeFileSync(executable, Buffer.alloc(0), { flag: "wx" });
  }),
}));

// Linking the app bundle needs a Cargo build this suite does not have, and the
// launch it precedes is replaced, so linking is replaced too.
vi.mock("../../web/e2e/bundled-app", async (original) => ({
  ...await original<typeof import("../../web/e2e/bundled-app")>(),
  linkBundle: vi.fn(),
}));

/** Windows gets the built no-op opener in the private root; elsewhere /usr/bin/true needs no program. */
function expectOpenerCopied(run: Isolated): void {
  if (process.platform === "win32") expect(copyFixtureShim).toHaveBeenCalledWith("noop", path.join(run.root, "bin", "hide-open.exe"));
  else expect(copyFixtureShim).not.toHaveBeenCalled();
}

let root: string;
let runs: Isolated[];
let children: ChildProcess[];
beforeEach(() => {
  vi.stubEnv(FUSED_ELECTRON_ENV, "/fused/electron");
  root = fs.mkdtempSync(path.join(os.tmpdir(), "hide-fixture-regression-"));
  vi.spyOn(os, "tmpdir").mockReturnValue(root);
  boundary.launch.mockReset();
  vi.mocked(spawnSync).mockClear();
  vi.mocked(endWindowsProcesses).mockClear();
  vi.mocked(copyFixtureShim).mockClear();
  runs = [];
  children = [];
});
afterEach(() => {
  // These are inert handles, not real PIDs. Restore the reported exit state
  // so the same recovery entrypoint can clean every test's retained home.
  for (const child of children) reportExit(child);
  for (const run of runs) run.cleanup();
  vi.restoreAllMocks();
  fs.rmSync(root, { recursive: true, force: true });
});

function privateHome(label: string): Isolated {
  const run = isolate({ bin: "/unused/herdr", socket: path.join(root, "unused.sock") }, label);
  runs.push(run);
  return run;
}

function reportExit(child: ChildProcess, signal: NodeJS.Signals | null = null): void {
  Object.assign(child, { exitCode: signal ? null : 0, signalCode: signal });
}

function candidate(close: (child: ChildProcess) => Promise<void>): ChildProcess {
  const child = new ChildProcess();
  children.push(child);
  boundary.launch.mockResolvedValueOnce({ process: () => child, firstWindow: async () => ({}), close: () => close(child) });
  return child;
}

async function teardown(use: () => Promise<void>): Promise<AggregateError | undefined> {
  try { await boundary.guard!({}, use, { tags: [] }); }
  catch (error) {
    expect(error).toBeInstanceOf(AggregateError);
    return error as AggregateError;
  }
}

test("rejected close preserves only the live candidate's home and permits recovery after exit", async () => {
  const closeFailure = new Error("injected Electron close failure");
  const live = candidate(async () => { throw closeFailure; });
  candidate(async (child) => { reportExit(child); });
  const run = privateHome("live");
  const other = privateHome("exited");
  const error = await teardown(async () => {
    await launch(run.env);
    await launch(other.env);
  });
  expect(live.exitCode).toBeNull();
  // The live candidate's home is preserved, so nothing is ended or removed for it.
  expect(endWindowsProcesses).not.toHaveBeenCalledWith([], run.root);
  expect(error?.errors).toContain(closeFailure);
  expect(fs.existsSync(run.env.HOME!)).toBe(true);
  expect(fs.existsSync(other.root)).toBe(false);
  const recovery = error?.errors.find((item: Error) => item.message.includes(run.root)) as Error;
  expect(recovery.message).toMatch(/confirm.*exit.*cleanup/i);
  // Repeated cleanup and a later test cannot forget the still-live handle.
  expect(() => run.cleanup()).toThrow(run.root);
  expect(fs.existsSync(run.root)).toBe(true);
  reportExit(live);
  run.cleanup();
  run.cleanup();
  expect(fs.existsSync(run.root)).toBe(false);
});

test("a rejected close after confirmed candidate exit still cleans home and reports the close error", async () => {
  const closeFailure = new Error("injected close response failure after exit");
  candidate(async (child) => { reportExit(child); throw closeFailure; });
  const run = privateHome("exited-close-error");
  const error = await teardown(async () => { await launch(run.env); });
  expect(error?.errors).toEqual([closeFailure]);
  expect(fs.existsSync(run.root)).toBe(false);
  expectRootEnded(run);
  expectOpenerCopied(run);
});

/** The exit Node reports for `child`: its exit code, then the `exit` event. */
function exitLater(child: ChildProcess): void {
  setImmediate(() => {
    reportExit(child);
    child.emit("exit", 0, null);
  });
}

// Playwright's close returns once the app's debugging pipe closes, and on
// Windows the recorded process (the cmd.exe Playwright starts the app under)
// exits after that (with-hide/hide-private#244).
test("a close that returns before its candidate exits waits for that exit, then the home goes", async () => {
  candidate(async (child) => { exitLater(child); });
  const run = privateHome("late-exit");
  const error = await teardown(async () => { await launch(run.env); });
  expect(error).toBeUndefined();
  expect(fs.existsSync(run.root)).toBe(false);
});

test("a candidate that never exits after a successful close fails the close by its pid and keeps the home", async () => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  try {
    const child = candidate(async () => {});
    Object.assign(child, { pid: 4242 });
    const run = privateHome("unconfirmed-exit");
    const ended = teardown(async () => { await launch(run.env); });
    await vi.advanceTimersByTimeAsync(10_000);
    const error = await ended;
    expect(error?.errors.some((item: Error) => item.message === "child 4242 did not exit within 10000 ms after app.close() returned")).toBe(true);
    expect(error?.errors.some((item: Error) => item.message.includes(run.root))).toBe(true);
    expect(fs.existsSync(run.env.HOME!)).toBe(true);
    reportExit(child, "SIGTERM");
    run.cleanup();
    expect(fs.existsSync(run.root)).toBe(false);
  } finally {
    vi.useRealTimers();
  }
});

test("manual cleanup rejects a live candidate before stopping any private service", async () => {
  candidate(async (child) => { reportExit(child); });
  const run = privateHome("manual-cleanup");
  const error = await teardown(async () => {
    await launch(run.env);
    expect(() => run.cleanup()).toThrow(run.root);
    expect(fs.existsSync(run.env.HOME!)).toBe(true);
    expect(spawnSync).not.toHaveBeenCalled();
  });
  expect(error).toBeUndefined();
  expect(fs.existsSync(run.root)).toBe(false);
});

test("pending launches retain HOME and the seventeenth candidate is refused before launch", async () => {
  const complete: (() => void)[] = [];
  boundary.launch.mockImplementation(() => new Promise((resolve) => {
    const child = new ChildProcess();
    children.push(child);
    complete.push(() => resolve({ process: () => child, firstWindow: async () => ({}), close: async () => { reportExit(child); } }));
  }));
  const run = privateHome("pending-candidates");
  const error = await teardown(async () => {
    const pending = Array.from({ length: 16 }, () => launch(run.env));
    try {
      expect(() => run.cleanup()).toThrow(run.root);
      expect(fs.existsSync(run.env.HOME!)).toBe(true);
      expect(spawnSync).not.toHaveBeenCalled();
      await expect(launch(run.env)).rejects.toThrow("16 live or launching candidates");
      expect(boundary.launch).toHaveBeenCalledTimes(16);
    } finally {
      for (const resolve of complete) resolve();
      await Promise.all(pending);
    }
  });
  expect(error).toBeUndefined();
  expect(fs.existsSync(run.root)).toBe(false);
});

test("a Herdr that is still running removes the root after it stops on Windows, and at once elsewhere", () => {
  const queued: (() => void)[] = [];
  const run = isolate({ bin: "/unused/herdr", socket: path.join(root, "unused.sock"), afterStop: (remove) => { queued.push(remove); } }, "after-stop");
  runs.push(run);
  run.cleanup();
  // A registered project's folder is held by the Herdr server's panes on Windows until it is gone.
  expect(queued).toHaveLength(process.platform === "win32" ? 1 : 0);
  expect(fs.existsSync(run.root)).toBe(process.platform === "win32");
  for (const remove of queued) remove();
  expect(fs.existsSync(run.root)).toBe(false);
});

test("an exception the host caught nothing for fails cleanup with its stack once the home is gone, unless the test caused it", () => {
  const record = (run: Isolated, message: string) => {
    const logs = path.join(run.env.HIDE_DESKTOP_USER_DATA_DIR!, "logs");
    fs.mkdirSync(logs, { recursive: true });
    fs.appendFileSync(path.join(logs, "desktop.log"), `${JSON.stringify({ event: "host.uncaught", kind: "uncaughtException", message, stack: `Error: ${message}\n    at quit (main.js:1:1)` })}\n`);
  };
  const run = privateHome("uncaught");
  record(run, "unexpected failure");
  expect(() => run.cleanup()).toThrow("uncaughtException: Error: unexpected failure\n    at quit (main.js:1:1)");
  expect(fs.existsSync(run.root)).toBe(false);
  // Reported once: the worker-exit cleanup does not report it again.
  expect(() => run.cleanup()).not.toThrow();
  const planted = privateHome("planted");
  planted.allowHostUncaught("planted");
  record(planted, "planted failure");
  expect(() => planted.cleanup()).not.toThrow();
  expect(fs.existsSync(planted.root)).toBe(false);
});
