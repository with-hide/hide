// When a unit test fails, one line says how its time went: the wall time
// beside the CPU time its worker spent and the longest the worker's event loop
// was held, and the machine's size and free memory. A test that took half a
// second on every other Windows run timed out at 5 s together with the files
// running beside it (with-hide/hide-private#354), and the run said nothing of
// whether the test worked that long, its worker got no CPU, or something held
// the loop. A passing test prints nothing.
import os from "node:os";
import { monitorEventLoopDelay } from "node:perf_hooks";
import { beforeEach } from "vitest";

const loop = monitorEventLoopDelay({ resolution: 10 });
loop.enable();

beforeEach((context) => {
  // Not `performance.now()`: a test's fake timers may replace it.
  const started = process.hrtime.bigint();
  const cpu = process.cpuUsage();
  loop.reset();
  context.onTestFailed(() => {
    const wall = Number(process.hrtime.bigint() - started) / 1e6;
    const used = process.cpuUsage(cpu);
    const load = process.platform === "win32" ? "" : `, load ${os.loadavg()[0]!.toFixed(1)}`;
    const mb = (bytes: number) => Math.round(bytes / 2 ** 20);
    console.log(
      `[unit timing] ${context.task.name}: wall ${wall.toFixed(0)} ms, worker cpu ${((used.user + used.system) / 1000).toFixed(0)} ms, ` +
        `longest event loop delay ${(loop.max / 1e6).toFixed(0)} ms; ${os.availableParallelism()} cpus${load}, ${mb(os.freemem())} of ${mb(os.totalmem())} MB free`,
    );
  });
});
