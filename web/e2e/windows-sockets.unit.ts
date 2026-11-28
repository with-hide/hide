// The TCP summary a failed Windows e2e attempt logs and the busiest processes
// a failed unit test logs, run against the real runner in the Windows check
// lane. Windows only, declared so Vitest lists them only there.
import { describe, expect, test } from "vitest";
import { windowsBusiestProcesses, windowsTcpSummary } from "./platform-fixture";

describe.runIf(process.platform === "win32")("on Windows", () => {
  test("a Windows TCP summary counts connections by state and by process name", { timeout: 30_000 }, () => {
    const [total, states, processes, ports, memory, ...rest] = windowsTcpSummary();
    expect(total).toMatch(/^TCP connections: \d+$/);
    expect(states).toMatch(/^by state: (\w+ \d+(, )?)*$/);
    expect(processes).toMatch(/^by process: .*/);
    expect(ports).toMatch(/^dynamic ports: \d+-\d+$/);
    expect(memory).toMatch(/^memory: nonpaged pool \d+ MB, available \d+ MB$/);
    expect(rest).toEqual([]);
  });

  test("the busiest Windows processes are named with their processor seconds and memory", { timeout: 30_000 }, () => {
    const [busiest, available, ...rest] = windowsBusiestProcesses();
    expect(busiest).toMatch(/^busiest: [^,]+ \d+ s \d+ MB(, [^,]+ \d+ s \d+ MB){7}$/);
    expect(available).toMatch(/^available memory: \d+ MB$/);
    expect(rest).toEqual([]);
  });
});
