// When a web unit test fails, what the machine was doing at that moment: its
// processor count and load and, on Windows, the processes that have used the
// most processor time (`windowsBusiestProcesses`). A test that ran in half a
// second on every other run timed out at 5 s on a Windows runner together with
// the files running beside it (with-hide/hide-private issue 354), and the run
// said nothing of what else held the machine. It runs in vitest's own process
// and only for a failed test: a passing run measures and prints nothing.
import os from "node:os";
import type { Reporter, TestCase } from "vitest/node";
import { windowsBusiestProcesses } from "./platform-fixture";

export default class UnitFailureReporter implements Reporter {
  onTestCaseResult(test: TestCase): void {
    if (test.result().state !== "failed") return;
    const load = process.platform === "win32" ? "" : `, load ${os.loadavg().map((value) => value.toFixed(1)).join(" ")}`;
    const lines = [`${os.availableParallelism()} cpus${load}, ${Math.round(os.freemem() / 2 ** 20)} of ${Math.round(os.totalmem() / 2 ** 20)} MB free`];
    if (process.platform === "win32") {
      try { lines.push(...windowsBusiestProcesses()); } catch (error) { lines.push(`processes unavailable: ${String(error)}`); }
    }
    for (const line of lines) process.stdout.write(`[unit failure] ${test.fullName} (${test.diagnostic()?.duration.toFixed(0) ?? "?"} ms): ${line}\n`);
  }
}
