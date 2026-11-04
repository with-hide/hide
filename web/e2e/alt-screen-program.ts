// A pager stand-in for the alternate-screen contract: it enters the alternate
// screen, draws numbered lines and a `:` prompt like `less`, appends every
// byte it reads from the terminal to a log, and leaves on `q`. The system
// `less` is not the same program on every runner (Windows has only the msys
// build, which reads the console its own way), and it cannot say what reached
// it; this one runs the same everywhere and its log can.
import fs from "node:fs";
import { fixtureProgram } from "./platform-fixture";
import { paneCommandLine, type PaneShell } from "./herdr-fixture";

// argv[2] is the log path. Raw mode makes each key reach the program as its
// bytes; a pipe has no raw mode and delivers the same bytes unchanged.
const SCRIPT = String.raw`
const fs = require("node:fs");
const log = process.argv[2];
if (!log) { console.error("alt-screen: no log path"); process.exit(64); }
fs.writeFileSync(log, "");
const rows = process.stdout.rows || 24;
let screen = "\x1b[?1049h\x1b[H";
for (let line = 1; line < rows; line++) screen += line + "\r\n";
screen += ":";
process.stdout.write(screen);
if (process.stdin.isTTY) process.stdin.setRawMode(true);
process.stdin.on("data", (chunk) => {
  fs.appendFileSync(log, chunk);
  if (chunk.includes(0x71)) process.stdout.write("\x1b[?1049l", () => process.exit(0));
});
`;

/**
 * The pager program made in `dir`, as the command line that runs it and
 * appends everything it reads to `log`; `bytesReceived` reads that log.
 */
export function altScreenProgram(dir: string, log: string, shell: PaneShell): { command: string } {
  const executable = fixtureProgram(dir, "alt-screen", SCRIPT);
  return { command: paneCommandLine(shell, [executable, log]) };
}

/** Every byte the pager has read so far, in order. */
export function bytesReceived(log: string): Buffer {
  return fs.readFileSync(log);
}
