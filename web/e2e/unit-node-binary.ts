// Reads the Node binary once, start to end, in vitest's own process before any
// test worker starts. Every test file runs in a new Node process, and on a
// Windows runner, where Node lives on the slow system disk, the first ones read
// that binary from disk a few kilobytes at a time while their first test's time
// limit ran. Read whole here, that cold read happens before any test's clock
// starts; where the binary is already in memory, as on a developer's machine, it
// takes milliseconds. This mitigates; whether that cold read caused the 12 to 18 s
// first-test stalls of with-hide/hide-private issue 354 is not confirmed.
import { closeSync, openSync, readSync } from "node:fs";

export default function readNodeBinary(): void {
  const chunk = Buffer.allocUnsafe(4 * 2 ** 20);
  const descriptor = openSync(process.execPath, "r");
  try {
    while (readSync(descriptor, chunk, 0, chunk.length, null) > 0);
  } finally {
    closeSync(descriptor);
  }
}
