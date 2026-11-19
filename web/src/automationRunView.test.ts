import { afterEach, describe, expect, it, vi } from "vitest";
import { viewRun } from "./automationRunView";

// PRD automations B15: the core holds a viewing 30 minutes unless it hears it again.
const CORE_HOLD_MS = 30 * 60_000;

describe("a shown run's terminal", () => {
  afterEach(() => vi.useRealTimers());

  it("is said again before the core's hold runs out, and no longer once the view leaves", () => {
    vi.useFakeTimers();
    const said: { on: boolean; at: number }[] = [];
    const stop = viewRun((on) => said.push({ on, at: Date.now() }));
    vi.advanceTimersByTime(2 * CORE_HOLD_MS);
    const ons = said.filter((word) => word.on).map((word) => word.at);
    expect(ons.length).toBeGreaterThan(1);
    expect(Math.max(...ons.slice(1).map((at, index) => at - ons[index]!))).toBeLessThan(CORE_HOLD_MS);
    stop();
    vi.advanceTimersByTime(CORE_HOLD_MS);
    expect(said.at(-1)?.on).toBe(false);
    expect(said.filter((word) => !word.on)).toHaveLength(1);
  });
});
