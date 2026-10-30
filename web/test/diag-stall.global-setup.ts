// Throwaway diagnosis for with-hide/hide-private issue 354, never merged.
// Every variant: vitest's own process logs each moment its event loop could not
// run a 100 ms timer for over 300 ms, to line up with the workers' stalls.
import { appendFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";

export default function setup() {
  const directory = process.env.DIAG_DIR;
  if (!directory) throw new Error("DIAG_DIR is required for the diagnosis global setup");
  mkdirSync(directory, { recursive: true });
  const sink = join(directory, `main-${process.pid}.log`);
  const cpuMs = () => { const usage = process.cpuUsage(); return (usage.user + usage.system) / 1000; };
  let last = performance.now();
  let lastCpu = cpuMs();
  const timer = setInterval(() => {
    const now = performance.now();
    const cpu = cpuMs();
    if (now - last > 400) appendFileSync(sink, `${new Date().toISOString()} main stalled ${(now - last - 100).toFixed(0)} ms, cpu ${(cpu - lastCpu).toFixed(0)} ms in it\n`);
    last = now;
    lastCpu = cpu;
  }, 100);
  return () => clearInterval(timer);
}
