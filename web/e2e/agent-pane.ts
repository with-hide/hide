// A pane that runs a listed Claude Code agent whose `hide` commands come
// from inside the agent's own process, as a real agent's do, on either
// engine: the fixture `claude` execs the spawn-provider shim, which reports
// its native session the way the pane's engine learns one (Herdr's
// integration report on Herdr, Hide's SessionStart hook on the core's own
// engine) and, on each `!` typed into the pane, runs a small driver that
// reads the next command from a file. Hide binds such a command to the
// agent's pane, so what it opens carries that agent as its opener.

import { expect } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import type { EngineServer } from "./engine-fixture";
import { connectStateDir } from "./hided-control.mjs";
import { fixtureExecutable } from "./platform-fixture";
import { copyFixtureShim } from "./shims/build";

/** One `hide` command's exit status and standard output. */
export type HideAnswer = { status: number; out: string };

/** Another program to run as the agent instead of `hide`, such as the hook helper, its standard input and environment. */
export type AgentRun = { program?: string; input?: string; env?: Record<string, string> };

/** What a pane's agent runner needs of its fixture: its root, and on Herdr the CLI a session is reported with; the own engine has none. */
export type AgentPaneFixture = Pick<EngineServer, "root"> & { bin?: string };

/** Types `text` into `pane` as the operator's keys, through the daemon whose state folder is `stateDir`. */
export async function typeInto(stateDir: string, pane: string, text: string): Promise<void> {
  const control = await connectStateDir(stateDir);
  try {
    control.send("key", { pane_id: pane, bytes_base64: Buffer.from(text, "utf8").toString("base64") });
  } finally {
    control.close();
  }
}

/**
 * Starts the agent in the shell of `pane`, typed at its prompt as the
 * operator starts one, and returns the way to run `hide` as it, against the
 * daemon whose state folder is `stateDir`. Waits until the agent has
 * reported its session.
 */
export async function agentPane(fixture: AgentPaneFixture, stateDir: string, pane: string): Promise<(args: string[], run?: AgentRun) => Promise<HideAnswer>> {
  const run = agentRunner(fixture, stateDir, pane);
  await typeInto(stateDir, pane, "claude\r");
  await expect.poll(() => fs.existsSync(reportedMarker(fixture.root, pane)), { message: `${pane} runs a Claude Code agent that reported its session`, timeout: 20_000 }).toBe(true);
  return run;
}

/** The file the session reporter writes once `pane`'s agent has reported its session. */
function reportedMarker(root: string, pane: string): string {
  return path.join(root, `session-reported-${pane.replaceAll(":", "-")}`);
}

/**
 * Makes every fixture `claude` started from now on run as such an agent, as a
 * Reopen's resumed one does, and returns the way to run `hide` as the agent
 * in `pane` once one runs there.
 */
export function agentRunner(fixture: AgentPaneFixture, stateDir: string, pane: string, kind: "claude" | "grok" = "claude"): (args: string[], run?: AgentRun) => Promise<HideAnswer> {
  const root = fixture.root;
  // An engine reads the agent from the program in the pane's foreground, so the runner takes the agent's name.
  const runner = path.join(root, "worker-bin", fixtureExecutable(kind));
  fs.mkdirSync(path.dirname(runner), { recursive: true });
  copyFixtureShim("spawn-provider", runner);
  fs.writeFileSync(path.join(root, "agent-pane-runner.config"), runner);
  const command = path.join(root, "agent-pane-command.json");
  const output = path.join(root, "agent-pane-output.json");
  const completed = path.join(root, "agent-pane-command.status");
  const driver = path.join(root, "agent-pane-driver.cjs");
  const reporter = path.join(root, "agent-pane-session.cjs");
  // Both suites run from their package folder (`web`, `desktop`), one below the repository.
  const hide = path.resolve("..", "target", "debug", fixtureExecutable("hide"));
  const hooks = path.resolve("..", "target", "debug", fixtureExecutable("hide-agent-hooks"));
  fs.writeFileSync(driver, `
const fs = require("node:fs");
const { spawnSync } = require("node:child_process");
const { args, program, input, env } = JSON.parse(fs.readFileSync(${JSON.stringify(command)}, "utf8"));
const ran = spawnSync(program ?? ${JSON.stringify(hide)}, args, {
  env: { ...process.env, HIDE_STATE_DIR: ${JSON.stringify(stateDir)}, ...env },
  input, encoding: "utf8", timeout: 30000, maxBuffer: 1 << 20,
});
fs.writeFileSync(${JSON.stringify(output)}, JSON.stringify({ status: ran.status ?? -1, out: ran.stdout ?? "" }));
`);
  // Run by the stand-in from inside the pane as \`<node> <reporter> <pane> <session>\`.
  const report = fixture.bin
    ? `[${JSON.stringify(fixture.bin)}, ["pane", "report-agent-session", pane, "--source", "herdr:claude", "--agent", "claude", "--agent-session-id", session, "--seq", "1"], {}]`
    : `[${JSON.stringify(hooks)}, ["hook", "--runtime", "claude-code", "--event", "SessionStart"], { input: JSON.stringify({ session_id: session, hook_event_name: "SessionStart", source: "startup", cwd: process.cwd() }) }]`;
  fs.writeFileSync(reporter, `
const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const [pane, session] = process.argv.slice(2);
const [program, args, options] = ${report};
const ran = spawnSync(program, args, { ...options, encoding: "utf8", timeout: 20000 });
if (ran.status !== 0) process.exit(1);
fs.writeFileSync(path.join(${JSON.stringify(root)}, "session-reported-" + pane.replaceAll(":", "-")), session);
`);
  fs.writeFileSync(path.join(root, "provider.config"), [process.execPath, reporter, completed, process.execPath, driver, ""].join("\n"));
  return async (args, run = {}) => {
    fs.writeFileSync(command, JSON.stringify({ args, ...run }));
    fs.rmSync(completed, { force: true });
    fs.rmSync(output, { force: true });
    await typeInto(stateDir, pane, "!");
    await expect.poll(() => (fs.existsSync(completed) ? fs.readFileSync(completed, "utf8").trim() : null),
      { message: `hide ${args.join(" ")} ran in ${pane}`, timeout: 40_000 }).toBe("0");
    return JSON.parse(fs.readFileSync(output, "utf8")) as HideAnswer;
  };
}
