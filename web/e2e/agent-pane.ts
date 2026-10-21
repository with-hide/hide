// A Herdr pane that runs a listed Claude Code agent whose `hide` commands
// come from inside the agent's own process, as a real agent's do: the
// fixture `claude` execs the spawn-provider shim, which reports the native
// session and, on each `!` typed into the pane, runs a small driver that
// reads the next command from a file. Hide binds such a command to the
// agent's pane, so what it opens carries that agent as its opener.

import { expect } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import type { HerdrFixture } from "./herdr-fixture";
import { fixtureExecutable } from "./platform-fixture";
import { copyFixtureShim } from "./shims/build";

/** One `hide` command's exit status and standard output. */
export type HideAnswer = { status: number; out: string };

/** Another program of this build to run as the agent instead of `hide`, such as the hook helper, and its standard input. */
export type AgentRun = { program?: string; input?: string };

/**
 * Starts the agent in `pane` of `herdr` and returns the way to run `hide`
 * as it, against the daemon whose state folder is `stateDir`. Waits until
 * Herdr lists the agent with its session.
 */
export async function agentPane(herdr: HerdrFixture, stateDir: string, pane: string): Promise<(args: string[], run?: AgentRun) => Promise<HideAnswer>> {
  const root = herdr.root;
  const runner = path.join(root, "worker-bin", fixtureExecutable("claude"));
  fs.mkdirSync(path.dirname(runner));
  copyFixtureShim("spawn-provider", runner);
  fs.writeFileSync(path.join(root, "agent-pane-runner.config"), runner);
  const command = path.join(root, "agent-pane-command.json");
  const output = path.join(root, "agent-pane-output.json");
  const completed = path.join(root, "agent-pane-command.status");
  const driver = path.join(root, "agent-pane-driver.cjs");
  // Both suites run from their package folder (`web`, `desktop`), one below the repository.
  const hide = path.resolve("..", "target", "debug", fixtureExecutable("hide"));
  fs.writeFileSync(driver, `
const fs = require("node:fs");
const { spawnSync } = require("node:child_process");
const { args, program, input } = JSON.parse(fs.readFileSync(${JSON.stringify(command)}, "utf8"));
const ran = spawnSync(program ?? ${JSON.stringify(hide)}, args, {
  env: { ...process.env, HIDE_STATE_DIR: ${JSON.stringify(stateDir)} },
  input, encoding: "utf8", timeout: 30000, maxBuffer: 1 << 20,
});
fs.writeFileSync(${JSON.stringify(output)}, JSON.stringify({ status: ran.status ?? -1, out: ran.stdout ?? "" }));
`);
  fs.writeFileSync(path.join(root, "provider.config"), [herdr.bin, completed, process.execPath, driver, ""].join("\n"));
  herdr.run(["agent", "start", "viewer", "--kind", "claude", "--pane", pane]);
  await expect.poll(() => {
    const listed = herdr.run(["agent", "list"]) as { result: { agents: { pane_id: string; agent_session?: { value: string } }[] } };
    return listed.result.agents.find((agent) => agent.pane_id === pane)?.agent_session?.value ?? null;
  }, { message: `${pane} runs a listed Claude Code agent`, timeout: 20_000 }).toBeTruthy();
  return async (args, run = {}) => {
    fs.writeFileSync(command, JSON.stringify({ args, ...run }));
    fs.rmSync(completed, { force: true });
    fs.rmSync(output, { force: true });
    execFileSync(herdr.bin, ["pane", "send-text", pane, "!"], { env: herdr.env, timeout: 10_000 });
    await expect.poll(() => (fs.existsSync(completed) ? fs.readFileSync(completed, "utf8").trim() : null),
      { message: `hide ${args.join(" ")} ran in ${pane}`, timeout: 40_000 }).toBe("0");
    return JSON.parse(fs.readFileSync(output, "utf8")) as HideAnswer;
  };
}
