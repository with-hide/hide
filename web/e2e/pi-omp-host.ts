// A stand-in for Pi and omp (PRD pi-omp-extension D-11): a small program run as
// `pi` or `omp` in a pane of the private engine that loads the extension Hide's
// kit wrote, the way both hosts load a file from their extensions folder, and
// emits its events with the shapes captured from Pi 1.0.4 and omp 18.7.0
// (`hide-agent-hooks/tests/fixtures/pi-extension/`). Hide's extension is what
// reports the session, so the pane is listed as a Pi or omp agent.
//
// The spec drives it through numbered command files in the pane's folder and
// reads each answer from the file of the same number; a command runs in the
// pane's own process, with the pane's environment, as an extension handler does.

import { expect } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { fixtureProgram } from "./platform-fixture";

const SCRIPT = String.raw`
const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");
const { pathToFileURL } = require("node:url");

const agent = path.basename(process.argv[1]);
const config = JSON.parse(fs.readFileSync(path.join(path.dirname(process.argv[1]), "pi-omp-host.json"), "utf8"));
// A pane on the core's own engine names itself HIDE_PANE_ID.
const pane = process.env.HIDE_PANE_ID;
const folder = path.join(config.dir, pane.replace(/[^A-Za-z0-9]/g, "-"));
fs.mkdirSync(folder, { recursive: true });
// The pane's hide reaches the fixture daemon, as the operator's reaches theirs.
process.env.HIDE_STATE_DIR = config.stateDir;

// Both hosts name a session by its file. omp's sit under a folder named after the checkout, so a deep checkout's
// path passes 250 bytes (PRD B5); this one does.
const sessions = path.join(folder, "sessions", agent === "omp" ? "-" + "deep-checkout-segment".repeat(12) + "-" : "-fixture-");
fs.mkdirSync(sessions, { recursive: true });
const session = path.join(sessions, "2026-10-08T20-05-41-366Z_01a11d1f-1f76-701d-87bb-b31dc28aab3b.jsonl");
fs.writeFileSync(session, "");

// The context each host hands a handler: the pane's TUI session, or one of omp's in-process subagents.
const main = { hasUI: true, mode: "tui", cwd: process.cwd(), sessionManager: { getSessionFile: () => session, getSessionId: () => "01a11d1f" },
  ...(agent === "omp" ? { agent: { kind: "main", id: "Main", name: "main", depth: 0 } } : {}) };
const sub = (id) => ({ hasUI: false, mode: "tui", cwd: process.cwd(),
  sessionManager: { getSessionFile: () => path.join(sessions, id + ".jsonl"), getSessionId: () => id },
  agent: { kind: "sub", id, name: "task", depth: 1, parentId: "Main" } });

function bind(module) {
  const handlers = new Map();
  module.default({ on(name, handler) { if (!handlers.has(name)) handlers.set(name, []); handlers.get(name).push(handler); } });
  return async (name, event, ctx) => {
    let result;
    for (const handler of handlers.get(name) ?? []) result = await handler({ type: name, ...event }, ctx);
    return result;
  };
}

let calls = 0;
let prepared = null;
const run = {
  // The host prepares the submission: Hide's hidden message, if any, is what before_agent_start returned.
  async prompt(emit, { text }) {
    const result = await emit("before_agent_start", { prompt: text, systemPrompt: "[omitted]" }, main);
    prepared = result?.message ?? null;
    return { message: prepared };
  },
  // The host writes the prompt, Hide's message and the reply, then ends the turn.
  async written(emit) {
    await emit("agent_start", {}, main);
    await emit("message_end", { message: { role: "user" } }, main);
    if (prepared) await emit("message_end", { message: { role: "custom", ...prepared } }, main);
    await emit("message_end", { message: { role: "assistant" } }, main);
    await emit("agent_end", { messages: "[omitted]" }, main);
    prepared = null;
    return {};
  },
  async tool(emit, { tool, input }) {
    const result = await emit("tool_call", { toolName: tool, toolCallId: "call_" + (calls += 1), input }, main);
    return result?.block ? { refused: String(result.reason) } : { ran: true };
  },
  // omp's task tool: the parent dispatches, then the subagent's own session runs in this process.
  async spawn(emit, { id }) {
    await emit("before_subagent_spawn", { agent: "task", invocationKind: "task", spawnKey: id }, main);
    return {};
  },
  async sub(emit, { id, step, text }) {
    const binding = subs.get(id) ?? subs.set(id, bind(extension)).get(id);
    if (step === "start") { await binding("session_start", {}, sub(id)); await binding("agent_start", {}, sub(id)); return {}; }
    if (step === "prompt") return { message: (await binding("before_agent_start", { prompt: text }, sub(id)))?.message ?? null };
    await binding("agent_end", { messages: "[omitted]" }, sub(id));
    return {};
  },
  async shell(emit, { argv }) {
    try {
      return { stdout: execFileSync(argv[0], argv.slice(1), { encoding: "utf8", timeout: 20000 }) };
    } catch (error) {
      return { status: error.status, stdout: String(error.stdout || ""), stderr: String(error.stderr || "") };
    }
  },
};
const subs = new Map();
let extension;

(async () => {
  // The extension's own session report is how Hide learns the session.
  // The kit writes a .ts file that is plain JavaScript; Node reads it as the ES module both hosts load.
  const copy = path.join(folder, "hide.mjs");
  fs.copyFileSync(config.extension[agent], copy);
  extension = await import(pathToFileURL(copy).href);
  const emit = bind(extension);
  await emit("session_start", { reason: "startup" }, main);
  fs.writeFileSync(path.join(folder, "ready.json.part"), JSON.stringify({ agent, session }));
  fs.renameSync(path.join(folder, "ready.json.part"), path.join(folder, "ready.json"));
  process.stdout.write(agent + " stand-in ready\n");
  for (let next = 1; ; ) {
    const command = path.join(folder, "command-" + next + ".json");
    if (!fs.existsSync(command)) { await new Promise((resolve) => setTimeout(resolve, 25)); continue; }
    const request = JSON.parse(fs.readFileSync(command, "utf8"));
    const answer = await run[request.op](emit, request);
    fs.writeFileSync(path.join(folder, "answer-" + next + ".json.part"), JSON.stringify(answer));
    fs.renameSync(path.join(folder, "answer-" + next + ".json.part"), path.join(folder, "answer-" + next + ".json"));
    next += 1;
  }
})().catch((error) => {
  fs.writeFileSync(path.join(folder, "failed.txt"), String(error && error.stack || error));
  process.exit(1);
});
`;

export type HostConfig = { stateDir: string; extension: { pi: string; omp: string }; dir: string };

/** Writes the stand-in as `pi` and `omp` in `bin`, configured by `config`. */
export function installPiOmpHosts(bin: string, config: HostConfig): void {
  fs.writeFileSync(path.join(bin, "pi-omp-host.json"), JSON.stringify(config));
  fixtureProgram(bin, "pi", SCRIPT);
  fixtureProgram(bin, "omp", SCRIPT);
}

export type HostCommand =
  | { op: "prompt"; text: string }
  | { op: "written" }
  | { op: "tool"; tool: string; input: Record<string, unknown> }
  | { op: "spawn"; id: string }
  | { op: "sub"; id: string; step: "start" | "prompt" | "end"; text?: string }
  | { op: "shell"; argv: string[] };

/** One running stand-in: its session file and a way to run a command in it. */
export type Host = { agent: string; session: string; send: (command: HostCommand) => Promise<Record<string, unknown>> };

/** Waits for the stand-in in `pane` to have loaded the extension, and returns its driver. */
export async function piOmpHost(dir: string, pane: string): Promise<Host> {
  const folder = path.join(dir, pane.replace(/[^A-Za-z0-9]/g, "-"));
  const ready = path.join(folder, "ready.json");
  await expect.poll(() => fs.existsSync(ready) || (fs.existsSync(path.join(folder, "failed.txt")) ? fs.readFileSync(path.join(folder, "failed.txt"), "utf8") : false),
    { message: `the stand-in in ${pane} loaded the extension`, timeout: 20_000 }).toBe(true);
  const { agent, session } = JSON.parse(fs.readFileSync(ready, "utf8")) as { agent: string; session: string };
  let sent = 0;
  return {
    agent,
    session,
    send: async (command) => {
      sent += 1;
      const answer = path.join(folder, `answer-${sent}.json`);
      // Written whole, then renamed into place, so the stand-in never reads half a command.
      fs.writeFileSync(path.join(folder, `command-${sent}.json.part`), JSON.stringify(command));
      fs.renameSync(path.join(folder, `command-${sent}.json.part`), path.join(folder, `command-${sent}.json`));
      await expect.poll(() => fs.existsSync(answer), { message: `the stand-in in ${pane} answered ${command.op}`, timeout: 20_000 }).toBe(true);
      return JSON.parse(fs.readFileSync(answer, "utf8")) as Record<string, unknown>;
    },
  };
}
