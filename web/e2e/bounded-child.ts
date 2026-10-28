// A child a vitest file awaits instead of waiting for it synchronously, so the
// worker keeps answering vitest (with-hide/hide-private#48). It is bounded the
// way the synchronous call was, in time and in output, and on every way out
// (its deadline, too much output, a start error, the caller's abort) its whole
// tree is ended and its pipes closed before the promise settles, so a caller's
// `finally` removes its files only once nothing it started can still use them.
import { execFile, spawn } from "node:child_process";

export type BoundedResult = { status: number | null; signal: NodeJS.Signals | null; stdout: string; stderr: string };

export type Bounds = {
  /** How long the child may run before its tree is ended. */
  timeoutMs: number;
  /** How many bytes stdout and stderr may hold, each. */
  outputBytes: number;
  /** Ends the child's tree when the caller gives up first, as a test that timed out does. */
  signal?: AbortSignal;
};

/** Runs `executable` to its end within `bounds`; any other ending rejects, with the output so far, after the tree is gone. */
export function runBounded(executable: string, args: readonly string[], bounds: Bounds): Promise<BoundedResult> {
  return new Promise((resolve, reject) => {
    // A private process group on Unix, so ending it ends what the child started.
    const child = spawn(executable, args, { detached: process.platform !== "win32", windowsHide: true, stdio: ["ignore", "pipe", "pipe"] });
    const output = { stdout: "", stderr: "" };
    let failure: Error | undefined;
    let ending: Promise<void> = Promise.resolve();
    const fail = (error: Error) => {
      if (failure) return;
      failure = error;
      if (child.pid !== undefined) ending = endTree(child.pid);
      // A grandchild that inherited a pipe would hold `close` open after the tree is gone.
      child.stdout.destroy();
      child.stderr.destroy();
    };
    const timer = setTimeout(() => fail(new Error(`${executable} did not end within ${bounds.timeoutMs} ms`)), bounds.timeoutMs);
    const abort = () => fail(new Error(`${executable} was abandoned by its caller`, { cause: bounds.signal?.reason }));
    if (bounds.signal?.aborted) abort();
    else bounds.signal?.addEventListener("abort", abort, { once: true });
    for (const name of ["stdout", "stderr"] as const) {
      child[name].setEncoding("utf8").on("data", (chunk: string) => {
        if (Buffer.byteLength(output[name]) + Buffer.byteLength(chunk) > bounds.outputBytes) {
          fail(new Error(`${executable} wrote more than ${bounds.outputBytes} bytes to ${name}`));
          return;
        }
        output[name] += chunk;
      });
    }
    child.once("error", fail);
    child.once("close", (status, signal) => {
      clearTimeout(timer);
      bounds.signal?.removeEventListener("abort", abort);
      ending.then(
        () => (failure ? reject(Object.assign(failure, output)) : resolve({ status, signal, ...output })),
        (cleanup: unknown) => reject(new AggregateError([failure, cleanup], `${executable}'s tree could not be ended`)),
      );
    });
  });
}

/** Ends the tree `pid` leads: its Unix process group, or on Windows the process and every descendant it started. */
function endTree(pid: number): Promise<void> {
  if (process.platform !== "win32") {
    try {
      process.kill(-pid, "SIGKILL");
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ESRCH") return Promise.reject(error);
    }
    return Promise.resolve();
  }
  return new Promise((resolve, reject) => {
    execFile("taskkill.exe", ["/T", "/F", "/PID", String(pid)], { timeout: 10_000, windowsHide: true }, (error, _stdout, stderr) => {
      // 128: the process had already ended.
      if (!error || error.code === 128) resolve();
      else reject(new Error(`taskkill could not end ${pid}: ${stderr || error.message}`, { cause: error }));
    });
  });
}
