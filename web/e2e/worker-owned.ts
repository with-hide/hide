// A fixture process belongs to the Playwright worker that started it. A test
// that fails still runs its own `finally`, but a worker that dies first (an
// unhandled error event, the runner stopping it) skips every one of them, and
// the private Herdr server or hided it started lives on under launchd with its
// temporary directory. Every stop registered here also runs when the worker
// exits, so nothing a worker started outlives it.

const running = new Set<() => void>();

/** Runs every stop still owned, as the worker's exit does; exported for the test of that exit. */
export function stopEveryOwned(): void {
  for (const stop of [...running]) {
    try {
      stop();
    } catch (error) {
      console.error("e2e fixture stop failed at worker exit", error);
    }
  }
}

process.once("exit", stopEveryOwned);

/**
 * `stop`, run at most once, and at worker exit if nothing ran it before.
 * `disown` leaves it to the caller alone, for a process whose directory a
 * successor takes over (a daemon restart reuses it).
 */
export function ownUntilWorkerExit(stop: () => void): { stop: () => void; disown: () => void } {
  let done = false;
  const owned = () => {
    running.delete(owned);
    if (done) return;
    done = true;
    stop();
  };
  running.add(owned);
  return { stop: owned, disown: () => void running.delete(owned) };
}

/**
 * Runs `cleanup` after `primary` failed and returns `primary` for the caller
 * to throw. A cleanup that also fails is appended to its message: Playwright
 * prints the message and the stack frames, so the original failure keeps its
 * text and location and the cleanup failure is reported under it.
 */
export function afterCleanup(primary: unknown, cleanup: () => void): unknown {
  try {
    cleanup();
  } catch (secondary) {
    const detail = `cleanup after it also failed: ${secondary instanceof Error ? secondary.message : String(secondary)}`;
    if (!(primary instanceof Error)) return new Error(`${String(primary)}\n\n${detail}`, { cause: secondary });
    try {
      primary.message = `${primary.message}\n\n${detail}`;
    } catch {
      return new Error(`${primary.message}\n\n${detail}`, { cause: primary });
    }
  }
  return primary;
}
