// Which engine a run's daemons put their panes on (`HIDE_E2E_ENGINE`), read
// in one place by the fixtures and by the app bundle, which carries the pane
// holder only for the core's own engine.

export type E2eEngine = "herdr" | "own";

/** The engine HIDE_E2E_ENGINE names; unset is `herdr`, anything else fails the run. */
export function e2eEngine(): E2eEngine {
  const value = process.env.HIDE_E2E_ENGINE ?? "herdr";
  if (value === "herdr" || value === "own") return value;
  throw new Error(`HIDE_E2E_ENGINE must be herdr or own, not ${JSON.stringify(value)}`);
}
