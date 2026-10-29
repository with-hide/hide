// Ending a fixture's process tree, with nothing but Node under it, so the
// shim build, which compiles `hide-processes`, can use it too. On Unix a tree
// is a private process group; on Windows it is the processes `hide-processes`
// follows by identity, given its program's path.
import { execFileSync, spawnSync } from "node:child_process";

/** A Windows process, named by its id and creation time so a reused id is never taken for it. */
export type WindowsProcess = { id: number; created: string };

/**
 * `pid` and its live descendants, as `tool` (`hide-processes`) lists them.
 * Windows keeps a running executable and a process's working folder locked,
 * and a pane's processes can outlive the Herdr server that started them, so a
 * fixture lists its tree before it stops the server and ends what is left with
 * `endWindowsTree`.
 */
export function listWindowsTree(tool: string, pid: number): WindowsProcess[] {
  const listed = execFileSync(tool, ["tree", String(pid)], { encoding: "utf8", timeout: 30_000, windowsHide: true });
  return listed.split(/\r?\n/).filter(Boolean).map((line) => {
    const [id, created] = line.split(" ");
    return { id: Number(id), created: created! };
  });
}

/**
 * Ends `owned`, every process they started, and, given `root`, every process
 * started from an executable under it (the fixture's shims, which hided also
 * runs), and returns once none is left, so the caller can delete `root`.
 * What an owned process started is found from its identity, so a child it
 * started after `owned` was listed is ended even once its parent is gone: an
 * owner killed while it starts a child leaves that child behind, and on
 * Windows a child started suspended and not yet in its job object (hided's
 * `OwnedChild::spawn`) stays suspended for good and keeps its executable
 * locked, where the path check did not find it. Throws naming the survivors
 * after five seconds.
 */
export function endWindowsTree(tool: string, owned: WindowsProcess[], root?: string): void {
  // The folder's trailing separator keeps a sibling whose name starts the same from matching.
  const folder = root ? `${root.replace(/[\\/]+$/, "")}\\` : "-";
  try {
    execFileSync(tool, ["end", folder, ...owned.map((item) => `${item.id}:${item.created}`)], { encoding: "utf8", timeout: 30_000, windowsHide: true });
  } catch (error) {
    // The program names the survivors on stderr, which is the reason.
    const stderr = String((error as { stderr?: unknown }).stderr ?? "").trim();
    throw new Error(stderr || String(error), { cause: error });
  }
}

/** Ends the private Unix process group `pid` leads, including a wrapper's children, and returns once none of it is left. */
export function endProcessGroup(pid: number): void {
  try { process.kill(-pid, "SIGKILL"); } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ESRCH") throw error;
  }
  const deadline = Date.now() + 5_000;
  for (;;) {
    const remaining = deadline - Date.now();
    if (remaining <= 0) throw new Error(`process group ${pid} did not exit`);
    const listed = spawnSync("/bin/ps", ["-A", "-o", "pgid=,stat="], { encoding: "utf8", timeout: remaining });
    if (listed.error || listed.status !== 0) throw new Error(`cannot confirm process group ${pid} exit: ${listed.error ?? listed.stderr}`);
    // A zombie has exited and cannot use the root; Node reaps it after this
    // synchronous teardown, or the OS does when the worker itself exits.
    const live = listed.stdout.trim().split(/\r?\n/).some((line) => {
      const [group, state] = line.trim().split(/\s+/);
      return Number(group) === pid && !state?.startsWith("Z");
    });
    if (!live) return;
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, Math.min(25, remaining));
  }
}
