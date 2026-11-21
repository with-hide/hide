// The spawn stand-in owns its configuration format. Web and desktop fixtures
// pass named fields here; the journey calls the same --write-config entrypoint.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fixtureExecutable } from "./platform-fixture";
import { fixtureShimPath } from "./shims/build";

export function configureSpawnProvider(root: string, fields: { reporter: string; completed: string; command: string[] }): void {
  execFileSync(fixtureShimPath("spawn-provider"), ["--write-config", path.join(root, "provider.config"), process.execPath,
    fields.reporter, fields.completed, ...fields.command], { timeout: 5_000 });
}

/** A session report made by the actual pane process, followed by its report-completion marker. */
export function writeSessionReporter(fixture: { root: string }): string {
  const reporter = path.join(fixture.root, "agent-pane-session.cjs");
  const hooks = path.resolve("..", "target", "debug", fixtureExecutable("hide-agent-hooks"));
  const report = `[${JSON.stringify(hooks)}, ["hook", "--runtime", "claude-code", "--event", "SessionStart"], { input: JSON.stringify({ session_id: session, hook_event_name: "SessionStart", source: "startup", cwd: process.cwd() }) }]`;
  fs.writeFileSync(reporter, `
const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const [pane, session] = process.argv.slice(2);
const [program, args, options] = ${report};
const ran = spawnSync(program, args, { ...options, encoding: "utf8", timeout: 20000 });
if (ran.status !== 0) process.exit(1);
fs.writeFileSync(path.join(${JSON.stringify(fixture.root)}, "session-reported-" + pane.replaceAll(":", "-")), session);
`);
  return reporter;
}
