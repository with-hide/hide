// How a native spec ends: its first failure stays the test's failure, and a step that
// also failed is named beside it (docs/TESTING.md, Cleanup never hides the first failure).
import { afterCleanup } from "../../web/e2e/worker-owned";

/** One teardown step: what it does, in words a reader of the report can place, and the call. */
export type TeardownStep = readonly [step: string, run: () => unknown];

/**
 * A failure named by the step it came from. A spec records the failures it goes on
 * from (a capture that did not land) with this, so the report names the side.
 */
export function inStep(step: string, error: unknown): Error {
  return new Error(`${step}: ${error instanceof Error ? error.message : String(error)}`, { cause: error });
}

/**
 * Runs every `teardown` step, each after the one before it whether or not that one
 * failed, and throws the first of `failures` and the failed steps. Each later failure
 * is appended to its message by `afterCleanup`, so the report keeps the first
 * failure's text and location, which is the `expect` diagnostic the flaky report
 * classifies by, and names every step that also failed. With nothing failed it
 * returns.
 *
 * A spec used to throw one `AggregateError` over all of them, whose message said only
 * that "a fixture or teardown failed": the report could not say which, and the located
 * assertion sat in a nested error no report line reads.
 */
export async function finishNative(failures: unknown[], teardown: readonly TeardownStep[]): Promise<void> {
  const all = [...failures];
  for (const [step, run] of teardown) {
    try { await run(); } catch (error) { all.push(inStep(step, error)); }
  }
  if (!all.length) return;
  let primary = all[0];
  for (const secondary of all.slice(1)) primary = afterCleanup(primary, () => { throw secondary; });
  throw primary;
}
