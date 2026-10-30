// Reads the Node binary once, start to end, in vitest's own process before any
// test worker starts. Every test file runs in a new Node process, and the first
// ones paged that binary in from the system disk a few kilobytes at a time: on a
// Windows runner, where Node lives on the slow system disk, a first test waited
// there for 12 to 18 s inside its 5 s timeout (with-hide/hide-private issue 354).
// Read whole, the binary is in memory before any worker asks for a page of it;
// where it already is, as on a developer's machine, the read takes milliseconds.
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
