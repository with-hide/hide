// A child a vitest file awaits instead of waiting for it synchronously, so the
// worker keeps answering vitest (with-hide/hide-private#48). It is bounded the
// way the synchronous call was, in time and in output. However it ends (by
// itself, at its deadline, with too much output, a start error or the caller's
// abort) its whole tree is ended before the promise settles: its first
// process's exit says nothing of what that process started, and on Windows
// `close` comes with that exit even while a descendant holds the output. A
// tree whose end could not be confirmed is reported as such at once, so its
// caller keeps the files it may still hold. The tree belongs to the worker
// until then: a worker that exits first ends it too (`ownUntilWorkerExit`).
//
// The tree is owned from its start, never found later from snapshots: on Unix
// it is a private process group, and on Windows a job object `hide-processes
// run` puts the child in before its first instruction, so what a parent that
// exited first had started, or a child that exited before anyone looked, is
// still accounted for (with-hide/hide-private#324).
import { spawn, type ChildProcess } from "node:child_process";
import type { Readable } from "node:stream";
import { endProcessGroup } from "./process-tree";
import { ownUntilWorkerExit } from "./worker-owned";

export type BoundedResult = {
  status: number | null;
  signal: NodeJS.Signals | null;
  stdout: string;
  stderr: string;
  /** How many processes the tree had in all, counted where a Windows job owned it. */
  processes?: number;
};

export type Bounds = {
  /** How long the child may run before its tree is ended. */
  timeoutMs: number;
  /** How many bytes stdout and stderr may hold, each. */
  outputBytes: number;
  /** Ends the child's tree when the caller gives up first, as a test that timed out does. */
  signal?: AbortSignal;
  /**
   * `hide-processes`, whose `run` owns a Windows tree in a job object. A
   * Windows run needs it; only its own first compile runs without it,
   * through `runAlone`.
   */
  processTool?: string;
  /** The child's environment; this process's own unless given. */
  env?: NodeJS.ProcessEnv;
};

/** A run that did not end by itself. Its tree is gone when `treeEnded`; only then may its caller remove the files the child used. */
export class BoundedChildError extends Error {
  constructor(message: string, readonly treeEnded: boolean, readonly stdout: string, readonly stderr: string, options?: ErrorOptions) {
    super(message, options);
  }
}

/** How a run's tree is ended. */
export type Tree = {
  /**
   * Ends what is left of the tree and settles once none of it is, with how
   * many processes it had when that is counted; rejects with `Unstarted`
   * when the program never ran, and otherwise when its end cannot be said.
   */
  end: () => Promise<number | undefined>;
  /** Ends what may be left at once, for the worker's exit, which cannot wait. */
  abandon: () => void;
};

/** A program that never ran: it left no tree, and its run fails as a start error does. */
export class Unstarted extends Error {}

/** How long a tree's end may take to be confirmed: `hide-processes` gives it five seconds, and starting it on a loaded runner takes some of the rest. */
const END_WITHIN_MS = 30_000;

/** Runs `executable` to its end within `bounds`; any other ending rejects with a `BoundedChildError`. */
export function runBounded(executable: string, args: readonly string[], bounds: Bounds): Promise<BoundedResult> {
  if (process.platform !== "win32") {
    // A private process group, so ending it ends what the child started.
    const child = spawn(executable, args, { detached: true, env: bounds.env, stdio: ["ignore", "pipe", "pipe"] });
    return supervise(executable, child, bounds, processGroup(child));
  }
  if (!bounds.processTool) return Promise.reject(new Error(`${executable} cannot be run on Windows without hide-processes to own its tree; only its own first compile runs without it (runAlone)`));
  const child = spawn(bounds.processTool, ["run", executable, ...args], { windowsHide: true, env: bounds.env, stdio: ["pipe", "pipe", "pipe", "pipe"] });
  return supervise(executable, child, bounds, windowsJob(executable, child));
}

/**
 * Runs a program that starts no other process, which only the first compile
 * on Windows needs: `hide-processes` is what it builds, so nothing owns a
 * tree yet, and each step it runs is a single process (`compileInSteps` in
 * shims/build.ts), whose own exit is the end of its tree. On Unix it is
 * `runBounded`.
 */
export function runAlone(executable: string, args: readonly string[], bounds: Omit<Bounds, "processTool">): Promise<BoundedResult> {
  if (process.platform !== "win32") return runBounded(executable, args, bounds);
  const child = spawn(executable, args, { windowsHide: true, env: bounds.env, stdio: ["ignore", "pipe", "pipe"] });
  const exited = new Promise<void>((resolve) => {
    if (child.exitCode !== null || child.signalCode !== null) resolve();
    else child.once("exit", () => resolve());
  });
  return supervise(executable, child, bounds, {
    end: async () => {
      if (child.exitCode === null && child.signalCode === null) child.kill();
      await within(exited, `${executable} did not exit`);
      return undefined;
    },
    abandon: () => void child.kill(),
  });
}

/**
 * The job `hide-processes run` owns `child`'s tree in: it ends the job when
 * the program exits or its input ends, and reports on descriptor 3 once none
 * of it is left. Killing it takes the job with it.
 */
function windowsJob(executable: string, child: ChildProcess): Tree {
  let reported = "";
  const report = child.stdio[3] as Readable | null | undefined;
  report?.setEncoding("utf8").on("data", (chunk: string) => {
    reported += chunk;
  });
  // Its report is complete once the descriptor closes, at its exit.
  const done = new Promise<void>((resolve) => {
    if (!report) resolve();
    else report.once("close", () => resolve());
  });
  return {
    end: async () => {
      // Its input ending is the caller's end; once it has exited this changes nothing.
      child.stdin?.destroy();
      await within(done, `hide-processes did not report the end of ${executable}'s tree`);
      const [kind, ...fields] = reported.trim().split(" ");
      if (kind === "ended") return Number(fields[1]);
      if (kind === "survivors") throw new Error(`${fields.length - 1} of its ${fields[0]} processes were still running after 5 s: ${fields.slice(1).join(", ")}`);
      if (kind === "unstarted") throw new Unstarted(`${executable} did not start: ${fields.join(" ")}`);
      throw new Error(`hide-processes ended without a report (status ${child.exitCode}, signal ${child.signalCode})${reported ? `: ${reported.trim()}` : ""}`);
    },
    // Its input closes with this process anyway, however it exits; closing it
    // now has it end the job and report, so a run still awaited settles.
    abandon: () => void child.stdin?.destroy(),
  };
}

function processGroup(child: ChildProcess): Tree {
  const pid = child.pid;
  return {
    end: async () => {
      endProcessGroup(pid!);
      return undefined;
    },
    abandon: () => endProcessGroup(pid!),
  };
}

/** `settled`, or a rejection naming `what` after `END_WITHIN_MS`. */
function within(settled: Promise<void>, what: string): Promise<void> {
  let timer: NodeJS.Timeout | undefined;
  const late = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${what} within ${END_WITHIN_MS} ms`)), END_WITHIN_MS);
  });
  return Promise.race([settled, late]).finally(() => clearTimeout(timer));
}

/**
 * Watches `child` within `bounds`, and however it ends ends its `tree`,
 * settling once the tree is gone or its end could not be confirmed. Exported
 * for the tests that give it a tree whose end fails.
 */
export function supervise(executable: string, child: ChildProcess, bounds: Bounds, tree: Tree): Promise<BoundedResult> {
  return new Promise((resolve, reject) => {
    const output = { stdout: "", stderr: "" };
    let settled = false;
    // Disowned once the tree is confirmed ended; until then the worker's exit ends it.
    const owned = child.pid === undefined ? undefined : ownUntilWorkerExit(tree.abandon);
    const finish = () => {
      settled = true;
      clearTimeout(timer);
      bounds.signal?.removeEventListener("abort", abort);
    };
    /** Ends the tree: how many processes it had, or why its end is not confirmed; a child that never started has none. */
    const endOwned = async (): Promise<{ processes?: number; unstarted?: Unstarted; unended?: string }> => {
      if (!owned) return {};
      try {
        const processes = await tree.end();
        owned.disown();
        return { processes };
      } catch (cleanup) {
        if (cleanup instanceof Unstarted) {
          owned.disown();
          return { unstarted: cleanup };
        }
        return { unended: `its tree was not confirmed ended: ${cleanup instanceof Error ? cleanup.message : String(cleanup)}` };
      }
    };
    const fail = (error: Error) => {
      if (settled) return;
      finish();
      void endOwned().then(({ unended }) => {
        // A process outside the tree that inherited a pipe would hold `close` open.
        child.stdout?.destroy();
        child.stderr?.destroy();
        const reason = unended ? `${error.message}; ${unended}` : error.message;
        reject(new BoundedChildError(reason, !unended, output.stdout, output.stderr, { cause: error }));
      });
    };
    const timer = setTimeout(() => fail(new Error(`${executable} did not end within ${bounds.timeoutMs} ms`)), bounds.timeoutMs);
    const abort = () => fail(new Error(`${executable} was abandoned by its caller`, { cause: bounds.signal?.reason }));
    if (bounds.signal?.aborted) abort();
    else bounds.signal?.addEventListener("abort", abort, { once: true });
    for (const name of ["stdout", "stderr"] as const) {
      child[name]?.setEncoding("utf8").on("data", (chunk: string) => {
        if (settled) return;
        if (Buffer.byteLength(output[name]) + Buffer.byteLength(chunk) > bounds.outputBytes) {
          fail(new Error(`${executable} wrote more than ${bounds.outputBytes} bytes to ${name}`));
          return;
        }
        output[name] += chunk;
      });
    }
    child.once("error", fail);
    child.once("close", (status, signal) => {
      if (settled) return;
      finish();
      void endOwned().then(({ processes, unstarted, unended }) => {
        if (unstarted) reject(new BoundedChildError(unstarted.message, true, output.stdout, output.stderr, { cause: unstarted }));
        else if (unended) reject(new BoundedChildError(`${executable} ended with status ${status}, signal ${signal}, but ${unended}`, false, output.stdout, output.stderr));
        else resolve({ status, signal, ...output, ...(processes === undefined ? {} : { processes }) });
      });
    });
  });
}
