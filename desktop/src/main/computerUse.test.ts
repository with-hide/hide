import { EventEmitter } from "node:events";
import { afterEach, expect, it, vi } from "vitest";
import { ComputerUse, COMPUTER_USE_POLL_MS } from "./computerUse";

class PowerMonitor extends EventEmitter {
  idle = 0;
  state = "active";
  getSystemIdleState() { return this.state; }
  getSystemIdleTime() { return this.idle; }
}

afterEach(() => vi.useRealTimers());

it("reports only transitions at ten minutes of system input, and releases its timer and listeners", () => {
  vi.useFakeTimers();
  const power = new PowerMonitor();
  const reports: boolean[] = [];
  const use = new ComputerUse(power, (value) => reports.push(value));
  use.start();
  power.idle = 599;
  vi.advanceTimersByTime(COMPUTER_USE_POLL_MS);
  expect(reports).toEqual([true]);
  power.idle = 600;
  vi.advanceTimersByTime(COMPUTER_USE_POLL_MS * 2);
  expect(reports).toEqual([true, false]);
  power.idle = 0;
  vi.advanceTimersByTime(COMPUTER_USE_POLL_MS);
  expect(reports).toEqual([true, false, true]);
  use.stop();
  expect(vi.getTimerCount()).toBe(0);
  expect(power.eventNames()).toEqual([]);
});

it("reports lock and sleep immediately, retains them across polling, and reads again on return", () => {
  vi.useFakeTimers();
  const power = new PowerMonitor();
  const reports: boolean[] = [];
  const use = new ComputerUse(power, (value) => reports.push(value));
  use.start();
  power.emit("lock-screen");
  vi.advanceTimersByTime(COMPUTER_USE_POLL_MS);
  expect(reports).toEqual([true, false]);
  power.emit("unlock-screen");
  power.emit("suspend");
  expect(reports).toEqual([true, false, true, false]);
  power.emit("resume");
  expect(reports).toEqual([true, false, true, false, true]);
  power.state = "locked";
  vi.advanceTimersByTime(COMPUTER_USE_POLL_MS);
  expect(use.current()).toBe(false);
  use.stop();
});

it("treats an unavailable idle source as in use, while still honoring sleep and lock events", () => {
  vi.useFakeTimers();
  const power = new PowerMonitor();
  power.state = "unknown";
  power.idle = 900;
  const use = new ComputerUse(power, () => undefined);
  use.start();
  expect(use.current()).toBe(true);
  power.emit("suspend");
  expect(use.current()).toBe(false);
  use.stop();
});
