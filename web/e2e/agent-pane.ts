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
import { configureSpawnProvider, writeSessionReporter } from "./spawn-provider";
import { focusPane, restoreFocusAfterInput, startedAgentIn, waitForInputGate } from "./daemon-actions";

/** One `hide` command's exit status and standard output. */
export type HideAnswer = { status: number; out: string };

/**
 * Another program to run as the agent instead of `hide`, such as the hook helper, its standard input and environment.
 * `hook` runs it as the agent's own hook rather than a tool call: the stand-in leaves its screen as it was instead of
 * returning to its composer, as Claude Code draws nothing for a hook.
 */
export type AgentRun = { program?: string; input?: string; env?: Record<string, string>; hook?: boolean };

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
  const control = await connectStateDir(stateDir);
  try {
    await focusPane(control, pane);
    await typeInto(stateDir, pane, "claude\r");
    const marker = reportedMarker(fixture.root, pane);
    await expect.poll(() => fs.existsSync(marker), { message: `${pane} ran its session reporter`, timeout: 20_000 }).toBe(true);
    const session = fs.readFileSync(marker, "utf8");
    await control.until((rest) => startedAgentIn(rest, pane)?.session_id === session || undefined, `${pane} on its reported native session, started`, 20_000);
  } finally {
    control.close();
  }
  return run;
}

/** The file the session reporter writes once `pane`'s agent has reported its session. */
export function reportedMarker(root: string, pane: string): string {
  return path.join(root, `session-reported-${pane.replaceAll(":", "-")}`);
}

const inputMarker = (root: string, pane: string) => path.join(root, `terminal-ready-${pane.replaceAll(":", "-")}`);

/** Before starting another program in the same pane, retire its previous program's readiness. */
export function prepareAgentInput(root: string, pane: string): void {
  fs.rmSync(inputMarker(root, pane), { force: true });
}

/** The current native stand-in has configured its terminal and can consume keys. */
export async function waitForAgentInput(root: string, pane: string): Promise<void> {
  const marker = inputMarker(root, pane);
  await expect.poll(() => fs.existsSync(marker), { message: `the agent in ${pane} reading its terminal`, timeout: 30_000 }).toBe(true);
}

/**
 * Makes every fixture `claude` started from now on run as such an agent, as a
 * Reopen's resumed one does, and returns the way to run `hide` as the agent
 * in `pane` once one runs there. Call before starting or reopening it: the
 * previous program's input readiness is retired here.
 */
export function agentRunner(fixture: AgentPaneFixture, stateDir: string, pane: string, kind: "claude" | "grok" = "claude"): (args: string[], run?: AgentRun) => Promise<HideAnswer> {
  prepareAgentInput(fixture.root, pane);
  return installAgentRunner(fixture, stateDir, kind)(pane);
}

/**
 * `agentRunner` before the agent's pane exists, for an agent Hide itself
 * starts: every fixture `claude` started from now on runs as such an agent,
 * and the answer gives the way to run `hide` as the one in a pane once it
 * runs there.
 */
export function installAgentRunner(fixture: AgentPaneFixture, stateDir: string, kind: "claude" | "grok" = "claude"): (pane: string) => (args: string[], run?: AgentRun) => Promise<HideAnswer> {
  const root = fixture.root;
  // An engine reads the agent from the program in the pane's foreground, so the runner takes the agent's name.
  const runner = path.join(root, "worker-bin", fixtureExecutable(kind));
  fs.mkdirSync(path.dirname(runner), { recursive: true });
  // Binding another pane reuses this run's immutable program. Rewriting an
  // executable already running can invalidate its code signature on macOS
  // or fail with a sharing violation on Windows.
  if (!fs.existsSync(runner)) copyFixtureShim("spawn-provider", runner);
  fs.writeFileSync(path.join(root, "agent-pane-runner.config"), runner);
  const command = path.join(root, "agent-pane-command.json");
  const output = path.join(root, "agent-pane-output.json");
  const completed = path.join(root, "agent-pane-command.status");
  const gate = path.join(root, "agent-pane-command.gate");
  const arrived = `${gate}.arrived`;
  const driver = path.join(root, "agent-pane-driver.cjs");
  // Both suites run from their package folder (`web`, `desktop`), one below the repository.
  const hide = path.resolve("..", "target", "debug", fixtureExecutable("hide"));
  fs.writeFileSync(driver, `
const fs = require("node:fs");
const { spawnSync } = require("node:child_process");
fs.writeFileSync(${JSON.stringify(arrived)}, "");
${waitForInputGate(gate, 15_000)}
const { args, program, input, env } = JSON.parse(fs.readFileSync(${JSON.stringify(command)}, "utf8"));
const ran = spawnSync(program ?? ${JSON.stringify(hide)}, args, {
  env: { ...process.env, HIDE_STATE_DIR: ${JSON.stringify(stateDir)}, ...env },
  input, encoding: "utf8", timeout: 30000, maxBuffer: 1 << 20,
});
fs.writeFileSync(${JSON.stringify(output)}, JSON.stringify({ status: ran.status ?? -1, out: ran.stdout ?? "" }));
`);
  configureSpawnProvider(root, { reporter: writeSessionReporter(fixture), completed, command: [process.execPath, driver] });
  return (pane) => async (args, run = {}) => {
    await waitForAgentInput(root, pane);
    fs.writeFileSync(command, JSON.stringify({ args, ...run }));
    fs.rmSync(completed, { force: true });
    fs.rmSync(output, { force: true });
    fs.rmSync(gate, { force: true });
    fs.rmSync(arrived, { force: true });
    // Read by the stand-in (`spawn-provider.c`, `ran_as_hook`) once the command has run.
    if (run.hook) fs.writeFileSync(`${completed}.hook`, "");
    else fs.rmSync(`${completed}.hook`, { force: true });
    const control = await connectStateDir(stateDir);
    try {
      await restoreFocusAfterInput(control, pane, () => typeInto(stateDir, pane, "!"), arrived);
      fs.writeFileSync(gate, "");
    } finally {
      control.close();
    }
    await expect.poll(() => (fs.existsSync(completed) ? fs.readFileSync(completed, "utf8").trim() : null),
      { message: `hide ${args.join(" ")} ran in ${pane}`, timeout: 40_000 }).toBe("0");
    return JSON.parse(fs.readFileSync(output, "utf8")) as HideAnswer;
  };
}
