import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fixtureProgram, fixtureToolPath } from "./platform-fixture";

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** A reset the way the CLI prints it: `Oct 1 at 3pm (UTC)`, with the year only when it differs. */
function printedReset(at: Date, now: Date): string {
  const year = at.getUTCFullYear() === now.getUTCFullYear() ? "" : `, ${at.getUTCFullYear()}`;
  const hour = at.getUTCHours();
  return `${MONTHS[at.getUTCMonth()]} ${at.getUTCDate()}${year} at ${hour % 12 === 0 ? 12 : hour % 12}${hour < 12 ? "am" : "pm"} (UTC)`;
}

/**
 * A `claude` on its own PATH directory that waits for `release()` and then
 * prints a `/usage` result frame: 62% of the week with a 6% Fable bucket,
 * both resetting on the whole hour five days and five hours from now. The
 * usage child only receives the account's login variables (`hide-platform`'s
 * `LOGIN_CHILD_VARIABLES`), so the script carries its paths itself.
 */
export function usageShim() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "hide-e2e-usage-"));
  const bin = path.join(root, "bin");
  fs.mkdirSync(bin);
  const gate = path.join(root, "released");
  const frame = path.join(root, "frame.json");
  const now = new Date();
  const hour = Math.floor(now.getTime() / 3_600_000) * 3_600_000;
  const weeklyResetAt = hour + (5 * 24 + 5) * 3_600_000;
  const reset = printedReset(new Date(weeklyResetAt), now);
  const result = [
    "You are currently using your subscription to power your Claude Code usage",
    "",
    `Current session: 4% used · resets ${printedReset(new Date(hour + 3 * 3_600_000), now)}`,
    `Current week (all models): 62% used · resets ${reset}`,
    `Current week (Fable): 6% used · resets ${reset}`,
  ].join("\n");
  fs.writeFileSync(frame, JSON.stringify({ type: "result", subtype: "success", is_error: false, result }));
  fixtureProgram(
    bin,
    "claude",
    `const fs = require("fs");
while (!fs.existsSync(${JSON.stringify(gate)})) Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 100);
process.stdout.write(fs.readFileSync(${JSON.stringify(frame)}));
`,
  );
  return {
    path: fixtureToolPath(bin),
    /** When both weekly windows reset, on the whole hour the CLI prints. */
    weeklyResetAt,
    release: () => fs.writeFileSync(gate, ""),
    cleanup: () => fs.rmSync(root, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 }),
  };
}

