// The stand-in for `hide-agent-hooks opencode <operation>` the plugin test
// runs: it logs the operation, its input and the pane it ran in, then answers
// from HIDE_OPENCODE_HELPER_ANSWERS. `delay_ms` holds the answer back, `exit`
// ends without one, `raw` prints it unparsed.

import { appendFileSync, readFileSync } from "node:fs";
import assert from "node:assert/strict";

// The generated extension must carry its route even if the host filters env.
assert.deepEqual(process.argv.slice(4), ["--state-dir-b64", "L2NhcHR1cmVkLXN0YXRl"]);

const operation = process.argv[3];
let stdin = "";
process.stdin.setEncoding("utf8");
for await (const chunk of process.stdin) stdin += chunk;
appendFileSync(
  process.env.HIDE_OPENCODE_HELPER_LOG,
  `${JSON.stringify({ operation, input: JSON.parse(stdin), env: { HERDR_PANE_ID: process.env.HERDR_PANE_ID } })}\n`,
);
const { delay_ms: delay, exit, raw, ...answer } =
  JSON.parse(readFileSync(process.env.HIDE_OPENCODE_HELPER_ANSWERS, "utf8"))[operation] ?? {};
if (delay) await new Promise((resolve) => setTimeout(resolve, delay));
if (exit !== undefined) process.exit(exit);
process.stdout.write(raw ?? JSON.stringify(answer));
