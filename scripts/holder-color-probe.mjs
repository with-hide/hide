// A pane's program that asks its terminal for the background color (OSC 11)
// and times each answer on its own clock, the way a CLI that detects its
// theme does: in raw mode, reading what arrives as input (docs/TESTING.md,
// The holder restart lane). It asks `count` times with nothing before the
// question, then `count` times right after writing 64 KiB of lines, and
// writes what it measured to `out` as JSON.
//
// Usage: node holder-color-probe.mjs <out.json> [count]

import fs from "node:fs";

const [out, countArg] = process.argv.slice(2);
const count = Number(countArg ?? 10);
const WAIT_MS = 2000;
const LINES = "x".repeat(63) + "\r\n";

process.stdin.setRawMode(true);
process.stdin.resume();
let heard = "";
let wake = null;
process.stdin.on("data", (data) => {
  heard += data.toString("latin1");
  if (wake) wake();
});

function ask(before) {
  return new Promise((resolve) => {
    heard = "";
    if (before) process.stdout.write(LINES.repeat(1024));
    const asked = performance.now();
    process.stdout.write("\x1b]11;?\x07");
    const done = (answered) => {
      clearTimeout(timer);
      wake = null;
      resolve({ ms: answered ? Math.round((performance.now() - asked) * 10) / 10 : null, raw: Buffer.from(heard, "latin1").toString("hex") });
    };
    const timer = setTimeout(() => done(false), WAIT_MS);
    wake = () => {
      if (heard.includes("11;rgb:") && (heard.includes("\x07") || heard.includes("\x1b\\"))) done(true);
    };
  });
}

const result = { platform: process.platform, idle: [], after_output: [] };
for (let index = 0; index < count; index++) result.idle.push(await ask(false));
for (let index = 0; index < count; index++) result.after_output.push(await ask(true));
// A late answer to the last question would show here.
await new Promise((resolve) => setTimeout(resolve, 500));
result.after_all = Buffer.from(heard, "latin1").toString("hex");
fs.writeFileSync(out, JSON.stringify(result));
process.stdin.setRawMode(false);
process.stdout.write("\x1b[2J\x1b[HPROBE DONE\r\n");
process.exit(0);
