// The stand-in for `hide-agent-hooks <pi|omp> <operation>` the extension test
// runs: it logs the agent, the operation, its input and the pane it ran in,
// then answers from HIDE_EXTENSION_HELPER_ANSWERS. `delay_ms` holds the answer
// back, `exit` ends without one, `raw` prints it unparsed.

import { appendFileSync, readFileSync } from "node:fs";
import assert from "node:assert/strict";

// The generated extension must carry its route even if the host filters env.
assert.deepEqual(process.argv.slice(4), ["--state-dir-b64", "L2NhcHR1cmVkLXN0YXRl"]);

const [agent, operation] = process.argv.slice(2);
let stdin = "";
process.stdin.setEncoding("utf8");
for await (const chunk of process.stdin) stdin += chunk;
// A recorded call is the test's barrier: capture its answer before publishing
// it, because the test may replace the answer file as soon as it sees the call.
const { delay_ms: delay, exit, raw, ...answer } =
  JSON.parse(readFileSync(process.env.HIDE_EXTENSION_HELPER_ANSWERS, "utf8"))[operation] ?? {};
appendFileSync(
  process.env.HIDE_EXTENSION_HELPER_LOG,
  `${JSON.stringify({ agent, operation, input: JSON.parse(stdin), env: { HERDR_PANE_ID: process.env.HERDR_PANE_ID } })}\n`,
);
if (delay) await new Promise((resolve) => setTimeout(resolve, delay));
if (exit !== undefined) process.exit(exit);
process.stdout.write(raw ?? JSON.stringify(answer));
