// The first import of the phone app in a test file, for the phone's own tests.

import { beforeAll, vi } from "vitest";

/**
 * How long a test file's first import of the phone app may take: it transforms the app's whole module graph, which on a Windows
 * runner beside the other test files ran past vitest's 10 s default for a hook, in a file that took 18 to 38 s there as a whole.
 * A hang guard for that import, above the slowest whole file seen, not a bound on how fast it should be.
 */
const FIRST_IMPORT_BUDGET_MS = 60_000;

/**
 * Imports the phone app once before the file's tests, with `socket` as the page's `WebSocket`, so the first test's budget is for
 * what it asserts and the import's cost is paid here, under a budget of its own.
 */
export function importAppFirst(socket: unknown): void {
  beforeAll(async () => {
    vi.stubGlobal("WebSocket", socket);
    try {
      await import("./App");
    } finally {
      vi.unstubAllGlobals();
    }
  }, FIRST_IMPORT_BUDGET_MS);
}
