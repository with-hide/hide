// The one way this host starts a child process (process practice 1).
//
// Every child is short-lived: a `hide` CLI verb, the login shell asked for
// its PATH, or VS Code being found or handed a folder (`editor.ts`). At most
// one runs at a time per runner, each has a hard timeout, and quitting the
// host kills the one in flight. Only the child's own pid is signalled:
// a daemon `hide connect` just started leads its own process group and is
// not the host's to end (desktop PRD D-03).

import { spawn, type ChildProcess } from "node:child_process";

/** The file name an executable has on this system: `name.exe` on Windows, `name` elsewhere. */
export function executableFile(name: string): string {
  return process.platform === "win32" ? `${name}.exe` : name;
}

/** Children in flight at once; a second request while one runs is refused. */
export const MAX_CHILDREN = 1;
/** Bytes kept from each stream; a verb answers one JSON line. */
export const MAX_OUTPUT_BYTES = 256 * 1024;

export type ChildResult = {
  code: number | null;
  signal: NodeJS.Signals | null;
  stdout: string;
  stderr: string;
  timedOut: boolean;
  /** The spawn itself failed (ENOENT, EACCES); the child never ran. A later stream error is not this. */
  spawnError: string | null;
};

export class ChildRunner {
  private current: ChildProcess | null = null;

  get busy(): boolean {
    return this.current !== null;
  }

  /** `env` is the child's whole environment; without it the child inherits this process's. */
  run(file: string, args: readonly string[], timeoutMs: number, env?: Record<string, string | undefined>): Promise<ChildResult> {
    if (this.current) return Promise.reject(new Error(`over budget: ${MAX_CHILDREN} child already running`));
    return new Promise((resolve) => {
      const child = spawn(file, args, { stdio: ["ignore", "pipe", "pipe"], env });
      this.current = child;
      let stdout = "";
      let stderr = "";
      let timedOut = false;
      let spawnError: string | null = null;
      const keep = (buffer: string, chunk: Buffer) => (buffer.length < MAX_OUTPUT_BYTES ? buffer + chunk.toString("utf8") : buffer);
      child.stdout?.on("data", (chunk: Buffer) => (stdout = keep(stdout, chunk)));
      child.stderr?.on("data", (chunk: Buffer) => (stderr = keep(stderr, chunk)));
      let settled = false;
      const settle = (code: number | null, signal: NodeJS.Signals | null) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        this.current = null;
        resolve({ code, signal, stdout, stderr, timedOut, spawnError });
      };
      const timer = setTimeout(() => {
        timedOut = true;
        child.kill("SIGKILL");
      }, timeoutMs);
      // A spawn that never started reports only `error`; one that ran ends in `close`.
      child.once("error", (error) => {
        if (child.pid !== undefined) return;
        spawnError = error.message;
        settle(null, null);
      });
      child.once("close", settle);
    });
  }

  /** Ends the child in flight, on quit. A child whose start failed has nothing to end. */
  stop(): void {
    if (this.current?.pid !== undefined) this.current.kill("SIGKILL");
  }
}

/**
 * The children this host starts and does not own: on Windows VS Code's
 * `Code.exe` handed a folder (`editor.ts`), the operator's editor window,
 * which the operator closes. It leads its own session and keeps no pipe to
 * this process.
 */
export function startDetached(file: string, args: readonly string[], env: Record<string, string | undefined>): Promise<{ pid: number } | { spawnError: string }> {
  return new Promise((resolve) => {
    const child = spawn(file, args, { detached: true, stdio: "ignore", env });
    child.once("error", (error) => resolve({ spawnError: error.message }));
    child.once("spawn", () => {
      child.unref();
      resolve({ pid: child.pid! });
    });
  });
}
