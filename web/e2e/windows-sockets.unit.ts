// The TCP summary a failed Windows e2e attempt logs, run against the real
// runner in the Windows check lane. Windows only.
import { expect, test } from "vitest";
import { windowsTcpSummary } from "./platform-fixture";

test("a Windows TCP summary counts connections by state and by process name", { timeout: 30_000 }, (context) => {
  if (process.platform !== "win32") return context.skip();
  const [total, states, processes, ports, memory, ...rest] = windowsTcpSummary();
  expect(total).toMatch(/^TCP connections: \d+$/);
  expect(states).toMatch(/^by state: (\w+ \d+(, )?)*$/);
  expect(processes).toMatch(/^by process: .*/);
  expect(ports).toMatch(/^dynamic ports: \d+-\d+$/);
  expect(memory).toMatch(/^memory: nonpaged pool \d+ MB, available \d+ MB$/);
  expect(rest).toEqual([]);
});
