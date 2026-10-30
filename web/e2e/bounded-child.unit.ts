// The ways a bounded child's tree could outlive what owns it: an end that
// fails, a first process that exits before what it started, a middle parent
// that exits before the grandchild it started, a child that exits before
// anything looked at it, a deadline that passes while the tree still runs, a
// worker that exits first, and the first compile on Windows, which runs
// before anything can own a tree (with-hide/hide-private#324). Each but the
// first runs real processes.
import { beforeAll, expect, test } from "vitest";
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { BoundedChildError, runAlone, runBounded, supervise, type BoundedResult, type Bounds } from "./bounded-child";
import { buildFixtureShims, compileInSteps, fixtureShimPath } from "./shims/build";
import { stopEveryOwned } from "./worker-owned";

const windows = process.platform === "win32";
const bounds: Bounds = { timeoutMs: 1_000, outputBytes: 64 * 1024 };

beforeAll(async () => {
  if (!windows) return;
  await buildFixtureShims(fileURLToPath(new URL("..", import.meta.url)), ["hide-processes", "noop"]);
  bounds.processTool = fixtureShimPath("hide-processes");
}, 120_000);

/**
 * A Node script whose child starts a grandchild, which writes its pid to
 * `pidFile` and waits; the child exits once the file is there, and the
 * script exits after the child. On Windows each is started detached, so no
 * Node job (libuv's own, which ends a child with its parent) ends the
 * grandchild for the tree's owner; on Unix that would leave the process group.
 */
const grandparent = (pidFile: string) => {
  const detached = JSON.stringify(windows);
  const grandchild = `require("node:fs").writeFileSync(${JSON.stringify(pidFile)}, String(process.pid)); setTimeout(() => {}, 60000);`;
  const child = `
const fs = require("node:fs");
require("node:child_process").spawn(process.execPath, ["-e", ${JSON.stringify(grandchild)}], { stdio: "ignore", detached: ${detached} });
const wait = () => fs.existsSync(${JSON.stringify(pidFile)}) ? process.exit(0) : setTimeout(wait, 20);
wait();
`;
  return `require("node:child_process").spawn(process.execPath, ["-e", ${JSON.stringify(child)}], { stdio: "ignore", detached: ${detached} }).once("exit", () => process.exit(0));`;
};

/**
 * A Node script that starts a second Node, which writes its pid to `pidFile`
 * and waits, holding the first one's output when `holdOutput`; the first waits
 * for that file, then exits unless `stay`.
 */
const parentOfHolder = (pidFile: string, { stay, holdOutput }: { stay: boolean; holdOutput: boolean }) => `
const { spawn } = require("node:child_process");
const fs = require("node:fs");
spawn(process.execPath, ["-e", ${JSON.stringify(`require("node:fs").writeFileSync(${JSON.stringify(pidFile)}, String(process.pid)); setTimeout(() => {}, 60000);`)}], { stdio: ${holdOutput ? '["ignore", "inherit", "inherit"]' : '"ignore"'} });
const wait = () => fs.existsSync(${JSON.stringify(pidFile)}) ? ${stay ? "setTimeout(() => {}, 60000)" : "process.exit(0)"} : setTimeout(wait, 20);
wait();
`;

const alive = (pid: number): boolean => {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ESRCH") return false;
    throw error;
  }
};

const scratch = () => fs.mkdtempSync(path.join(os.tmpdir(), "hide-bounded-"));

test("an end that fails is reported at once, as a tree not confirmed ended, and the worker still owns it", { timeout: 15_000 }, async () => {
  const child = spawn(process.execPath, ["-e", "setTimeout(() => {}, 60000)"], { stdio: ["ignore", "pipe", "pipe"] });
  let abandoned = 0;
  try {
    const started = Date.now();
    const failed = await supervise("stand-in", child, bounds, {
      end: () => Promise.reject(new Error("refused")),
      abandon: () => {
        abandoned += 1;
        child.kill("SIGKILL");
      },
    }).then(() => undefined, (error: unknown) => error);
    expect(failed).toBeInstanceOf(BoundedChildError);
    expect((failed as BoundedChildError).treeEnded).toBe(false);
    expect((failed as BoundedChildError).message).toContain("refused");
    // At its deadline, not at a `close` the live child never sends.
    expect(Date.now() - started).toBeLessThan(bounds.timeoutMs + 3_000);
    expect(alive(child.pid!)).toBe(true);
    stopEveryOwned();
    expect(abandoned).toBe(1);
  } finally {
    child.kill("SIGKILL");
  }
});

test("a first process that exits before what it started leaves none of it running", { timeout: 30_000 }, async () => {
  const dir = scratch();
  const pidFile = path.join(dir, "holder.pid");
  try {
    const done = await runBounded(process.execPath, ["-e", parentOfHolder(pidFile, { stay: false, holdOutput: false })], { ...bounds, timeoutMs: 10_000 });
    expect(done.status).toBe(0);
    // An ended Unix holder is reaped by init a moment later; until then it answers as a zombie.
    const holder = Number(fs.readFileSync(pidFile, "utf8"));
    await expect.poll(() => alive(holder), { timeout: 2_000 }).toBe(false);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("a grandchild whose parent exited first is ended with the tree", { timeout: 30_000 }, async () => {
  const dir = scratch();
  const pidFile = path.join(dir, "grandchild.pid");
  try {
    const done = await runBounded(process.execPath, ["-e", grandparent(pidFile)], { ...bounds, timeoutMs: 10_000 });
    expect(done.status).toBe(0);
    // The job counted the script, its child and the grandchild.
    if (windows) expect(done.processes).toBe(3);
    const grandchild = Number(fs.readFileSync(pidFile, "utf8"));
    await expect.poll(() => alive(grandchild), { timeout: 2_000 }).toBe(false);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("a child that exits before anything looked at it ends with its status", { timeout: 30_000 }, async () => {
  const done = await runBounded(windows ? fixtureShimPath("noop") : "/usr/bin/true", [], { ...bounds, timeoutMs: 10_000 });
  expect(done.status).toBe(0);
  if (windows) expect(done.processes).toBe(1);
});

test("a program that does not exist fails as a start error, with no tree left", { timeout: 30_000 }, async () => {
  const failed = await runBounded(path.join(os.tmpdir(), "hide-no-such-program"), [], { ...bounds, timeoutMs: 10_000 }).then(() => undefined, (error: unknown) => error);
  expect(failed).toBeInstanceOf(BoundedChildError);
  expect((failed as BoundedChildError).treeEnded).toBe(true);
});

test("a tree still running at its deadline is ended whole, with what holds its output", { timeout: 30_000 }, async () => {
  const dir = scratch();
  const pidFile = path.join(dir, "holder.pid");
  try {
    const failed = await runBounded(process.execPath, ["-e", parentOfHolder(pidFile, { stay: true, holdOutput: true })], { ...bounds, timeoutMs: 5_000 }).then(() => undefined, (error: unknown) => error);
    expect(failed).toBeInstanceOf(BoundedChildError);
    expect((failed as BoundedChildError).message).toContain("did not end within");
    expect((failed as BoundedChildError).treeEnded).toBe(true);
    const holder = Number(fs.readFileSync(pidFile, "utf8"));
    await expect.poll(() => alive(holder), { timeout: 2_000 }).toBe(false);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("a worker that exits first ends the tree", { timeout: 30_000 }, async () => {
  const dir = scratch();
  const pidFile = path.join(dir, "holder.pid");
  try {
    const run = runBounded(process.execPath, ["-e", parentOfHolder(pidFile, { stay: true, holdOutput: true })], { ...bounds, timeoutMs: 20_000 });
    await expect.poll(() => fs.existsSync(pidFile), { timeout: 10_000 }).toBe(true);
    const holder = Number(fs.readFileSync(pidFile, "utf8"));
    // What the worker's exit runs.
    stopEveryOwned();
    await expect.poll(() => alive(holder), { timeout: 2_000 }).toBe(false);
    // The ended child closes its pipes, so the run settles as one that ended.
    await expect(run).resolves.toBeDefined();
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("a first-compile step that does not end by its deadline is ended, with nothing yet to own a tree", { timeout: 30_000 }, async () => {
  const dir = scratch();
  const pidFile = path.join(dir, "compiler.pid");
  try {
    const hanging = `require("node:fs").writeFileSync(${JSON.stringify(pidFile)}, String(process.pid)); setTimeout(() => {}, 60000);`;
    const failed = await runAlone(process.execPath, ["-e", hanging], { timeoutMs: 5_000, outputBytes: bounds.outputBytes }).then(() => undefined, (error: unknown) => error);
    expect(failed).toBeInstanceOf(BoundedChildError);
    expect((failed as BoundedChildError).message).toContain("did not end within");
    expect((failed as BoundedChildError).treeEnded).toBe(true);
    const compiler = Number(fs.readFileSync(pidFile, "utf8"));
    await expect.poll(() => alive(compiler), { timeout: 2_000 }).toBe(false);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

// What lets the first compile on Windows run without a tree owner: each of
// its steps, counted here in a job, is one process, the compiler's exit is
// its tree's end, and the program it links runs. The variables that would
// have clang's driver start cc1 as a second process are set around it, as an
// account or a CI image may set them.
test("each step of the first compile on Windows runs a single process", { timeout: 120_000 }, async (context) => {
  if (!windows) return context.skip();
  const dir = scratch();
  const spawning = { CC_PRINT_PROC_STAT: "1", CCC_OVERRIDE_OPTIONS: "+-fno-integrated-cc1" };
  const before = Object.fromEntries(Object.keys(spawning).map((name) => [name, process.env[name]]));
  Object.assign(process.env, spawning);
  try {
    const output = path.join(dir, "noop.exe");
    const steps: BoundedResult[] = [];
    const linked = await compileInSteps("clang.exe", fileURLToPath(new URL("./shims/noop.c", import.meta.url)), output, async (program, args, env) => {
      const step = await runBounded(program, args, { ...bounds, timeoutMs: 60_000, outputBytes: 1024 * 1024, env });
      steps.push(step);
      return step;
    });
    expect(linked.status, linked.stdout + linked.stderr).toBe(0);
    expect(steps.map((step) => step.processes)).toEqual([1, 1, 1]);
    expect((await runBounded(output, [], { ...bounds, timeoutMs: 10_000 })).status).toBe(0);
  } finally {
    for (const [name, value] of Object.entries(before)) {
      if (value === undefined) delete process.env[name];
      else process.env[name] = value;
    }
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
