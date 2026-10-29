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
import { spawn, type ChildProcess } from "node:child_process";
import { endProcessGroup, endWindowsTree, listWindowsTree, type WindowsProcess } from "./process-tree";
import { ownUntilWorkerExit } from "./worker-owned";

export type BoundedResult = { status: number | null; signal: NodeJS.Signals | null; stdout: string; stderr: string };

export type Bounds = {
  /** How long the child may run before its tree is ended. */
  timeoutMs: number;
  /** How many bytes stdout and stderr may hold, each. */
  outputBytes: number;
  /** Ends the child's tree when the caller gives up first, as a test that timed out does. */
  signal?: AbortSignal;
  /**
   * `hide-processes`, which follows a Windows tree by identity. Without it a
   * Windows tree's end cannot be confirmed: that its first process is gone
   * says nothing of the processes it started.
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

/** Runs `executable` to its end within `bounds`; any other ending rejects with a `BoundedChildError`. */
export function runBounded(executable: string, args: readonly string[], bounds: Bounds): Promise<BoundedResult> {
  const windows = process.platform === "win32";
  // A private process group on Unix, so ending it ends what the child started.
  const child = spawn(executable, args, { detached: !windows, windowsHide: true, env: bounds.env, stdio: ["ignore", "pipe", "pipe"] });
  const pid = child.pid;
  if (!windows) return supervise(executable, child, bounds, () => endProcessGroup(pid!));
  // Listed while Node still holds the child's handle, so its id is not yet
  // anyone else's; what it starts later is found from this identity.
  let owned: WindowsProcess[] | undefined;
  let unlisted: unknown;
  if (pid !== undefined && bounds.processTool) {
    try {
      owned = listWindowsTree(bounds.processTool, pid).filter((item) => item.id === pid);
      if (owned.length === 0) unlisted = new Error(`${pid} was not listed while it ran`);
    } catch (error) {
      unlisted = error;
    }
  }
  const tool = bounds.processTool;
  return supervise(executable, child, bounds, () => {
    if (!tool) {
      // Only the compile of `hide-processes` itself runs without it (shims/build.ts):
      // nothing can follow that compiler's tree, so its own exit is all there is to go on.
      if (child.exitCode !== null || child.signalCode !== null) return;
      throw new Error("no hide-processes to follow the tree with");
    }
    if (unlisted !== undefined || !owned?.length) throw new Error(`the tree of ${pid} has no identity to follow`, { cause: unlisted });
    endWindowsTree(tool, owned);
  });
}

/**
 * Watches `child` within `bounds`, and however it ends runs `endTree`, which
 * returns once the tree is gone and throws when it cannot say so. Exported
 * for the tests that give it an `endTree` that fails.
 */
export function supervise(executable: string, child: ChildProcess, bounds: Bounds, endTree: () => void): Promise<BoundedResult> {
  return new Promise((resolve, reject) => {
    const output = { stdout: "", stderr: "" };
    let settled = false;
    let owned = child.pid === undefined ? undefined : ownUntilWorkerExit(endTree);
    const finish = () => {
      settled = true;
      clearTimeout(timer);
      bounds.signal?.removeEventListener("abort", abort);
    };
    /** Ends the tree, and says why not when it could not; a child that never started has none. */
    const endOwned = (): string | undefined => {
      if (!owned) return undefined;
      try {
        owned.stop();
        owned = undefined;
        return undefined;
      } catch (cleanup) {
        // Still the worker's to end when it exits.
        owned = ownUntilWorkerExit(endTree);
        return `its tree was not confirmed ended: ${cleanup instanceof Error ? cleanup.message : String(cleanup)}`;
      }
    };
    const fail = (error: Error) => {
      if (settled) return;
      finish();
      const unended = endOwned();
      // A process outside the tree that inherited a pipe would hold `close` open.
      child.stdout?.destroy();
      child.stderr?.destroy();
      const reason = unended ? `${error.message}; ${unended}` : error.message;
      reject(new BoundedChildError(reason, !unended, output.stdout, output.stderr, { cause: error }));
    };
    const timer = setTimeout(() => fail(new Error(`${executable} did not end within ${bounds.timeoutMs} ms`)), bounds.timeoutMs);
    const abort = () => fail(new Error(`${executable} was abandoned by its caller`, { cause: bounds.signal?.reason }));
    if (bounds.signal?.aborted) abort();
    else bounds.signal?.addEventListener("abort", abort, { once: true });
    for (const name of ["stdout", "stderr"] as const) {
      child[name]?.setEncoding("utf8").on("data", (chunk: string) => {
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
      const unended = endOwned();
      if (unended) reject(new BoundedChildError(`${executable} ended with status ${status}, signal ${signal}, but ${unended}`, false, output.stdout, output.stderr));
      else resolve({ status, signal, ...output });
    });
  });
}
