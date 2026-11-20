// A run's terminal shown in the automation screen (PRD automations B15): the
// window tells the core it shows, and tells it again well inside the core's
// hold (`VIEWING_HOLD_MS`, 30 minutes), so a terminal watched for hours stays
// attached and open; leaving the view says it no longer shows.

/** How often a shown run's terminal is said again: the core lets a viewing go 30 minutes after it last heard it. */
export const VIEWING_REPEAT_MS = 5 * 60_000;

/** Says the run shows now and every `VIEWING_REPEAT_MS` after; the returned stop says it no longer does. */
export function viewRun(say: (on: boolean) => void): () => void {
  say(true);
  const again = setInterval(() => say(true), VIEWING_REPEAT_MS);
  return () => {
    clearInterval(again);
    say(false);
  };
}
