// The ways a bounded child's tree could outlive what owns it: an end that
// fails, a first process that exits before what it started, a deadline that
// passes while the tree still runs, and a worker that exits first. Each but
// the first runs real Node processes.
import { beforeAll, expect, test } from "vitest";
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { BoundedChildError, runBounded, supervise, type Bounds } from "./bounded-child";
import { buildFixtureShims, fixtureShimPath } from "./shims/build";
import { stopEveryOwned } from "./worker-owned";

const bounds: Bounds = { timeoutMs: 1_000, outputBytes: 64 * 1024 };

beforeAll(async () => {
  if (process.platform !== "win32") return;
  await buildFixtureShims(fileURLToPath(new URL("..", import.meta.url)), ["hide-processes"]);
  bounds.processTool = fixtureShimPath("hide-processes");
}, 120_000);

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
  let ends = 0;
  try {
    const started = Date.now();
    const failed = await supervise("stand-in", child, bounds, () => {
      ends += 1;
      if (ends === 1) throw new Error("refused");
      child.kill("SIGKILL");
    }).then(() => undefined, (error: unknown) => error);
    expect(failed).toBeInstanceOf(BoundedChildError);
    expect((failed as BoundedChildError).treeEnded).toBe(false);
    expect((failed as BoundedChildError).message).toContain("refused");
    // At its deadline, not at a `close` the live child never sends.
    expect(Date.now() - started).toBeLessThan(bounds.timeoutMs + 3_000);
    expect(alive(child.pid!)).toBe(true);
    stopEveryOwned();
    expect(ends).toBe(2);
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
