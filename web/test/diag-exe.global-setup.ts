// Throwaway diagnosis for with-hide/hide-private issue 354, never merged.
// The "exe" variant: vitest's own process reads the Node binary once, start to end,
// before any worker starts, and records how long that took.
import { appendFileSync, closeSync, mkdirSync, openSync, readSync, statSync } from "node:fs";
import { join } from "node:path";

export default function setup() {
  const directory = process.env.DIAG_DIR;
  if (!directory) throw new Error("DIAG_DIR is required for the diagnosis global setup");
  mkdirSync(directory, { recursive: true });
  const sink = join(directory, `main-${process.pid}.log`);
  const start = performance.now();
  const buffer = Buffer.allocUnsafe(4 * 2 ** 20);
  const descriptor = openSync(process.execPath, "r");
  let total = 0;
  try {
    for (;;) {
      const read = readSync(descriptor, buffer, 0, buffer.length, null);
      if (read === 0) break;
      total += read;
    }
  } finally { closeSync(descriptor); }
  appendFileSync(sink, `${new Date().toISOString()} exe read ${process.execPath} ${(total / 2 ** 20).toFixed(0)} of ${(statSync(process.execPath).size / 2 ** 20).toFixed(0)} MB in ${(performance.now() - start).toFixed(0)} ms\n`);
}
