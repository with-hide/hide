#!/usr/bin/env node
// The measurement's end of the pane holders a run's own engine left, with the
// e2e fixture's one rule (web/e2e/pane-holders.mjs).
// usage: holders.mjs pids <state>   the holders and their programs, one pid a line
//        holders.mjs end <state>    ends them; exits 1 naming what still runs
//        holders.mjs left <state>   what still names <state>/holders; exits 1 if any
import { endPaneHolders, holderStrays, ownedHolders } from "../../web/e2e/pane-holders.mjs";

const [mode, stateDir] = process.argv.slice(2);
if (!stateDir || !["pids", "end", "left"].includes(mode)) {
  console.error("usage: holders.mjs pids|end|left <state folder>");
  process.exit(2);
}
try {
  if (mode === "pids") {
    for (const row of ownedHolders(stateDir)) console.log([row.holder, row.program].filter((pid) => pid !== undefined).join("\n"));
  } else if (mode === "end") {
    endPaneHolders(stateDir);
  } else {
    const strays = holderStrays(stateDir);
    for (const line of strays) console.log(line);
    if (strays.length > 0) process.exit(1);
  }
} catch (error) {
  console.error(error.message);
  process.exit(1);
}
