// A stand-in for OpenCode (PRD opencode-plugin D-13): a small program run as
// `opencode` in a pane of the private Herdr that loads the plugin Hide's kit
// wrote, the way OpenCode loads a file from its plugins folder, and calls its
// hooks with OpenCode 1.18.30's shapes (`hide-agent-hooks/tests/fixtures/
// opencode/events-1.18.30.json`). It tells Herdr what it is the way Herdr's
// own OpenCode integration does (`pane report-agent` with its session id), so
// Herdr lists the pane as an OpenCode agent.
//
// The spec drives it through numbered command files in the pane's folder and
// reads each answer from the file of the same number; a command runs in the
// pane's own process, with the pane's environment, as a plugin hook does.

import { expect } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { fixtureProgram } from "./platform-fixture";

const SCRIPT = String.raw`
const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");
const { pathToFileURL } = require("node:url");

const config = JSON.parse(fs.readFileSync(path.join(path.dirname(process.argv[1]), "opencode-host.json"), "utf8"));
const pane = process.env.HERDR_PANE_ID;
const folder = path.join(config.dir, pane.replace(/[^A-Za-z0-9]/g, "-"));
const session = "ses_" + pane.replace(/[^A-Za-z0-9]/g, "");
fs.mkdirSync(folder, { recursive: true });
// The pane's hide reaches the fixture daemon, as the operator's reaches theirs.
process.env.HIDE_STATE_DIR = config.stateDir;

// Herdr's OpenCode integration reports the pane's state and, from its TUI plugin, the session the pane
// selected (session_start_source "select"); Herdr takes them only once it has detected the agent in the
// pane, so the stand-in reports again until Herdr lists its session. Its sequence numbers are the
// integration's, time-based, so a report is never older than one Herdr already took.
const listed = () => JSON.parse(execFileSync(config.herdr, ["agent", "list"], { encoding: "utf8", timeout: 10000 }))
  .result.agents.find((agent) => agent.pane_id === pane)?.agent_session?.value;
function announce() {
  for (let attempt = 1; listed() !== session; attempt += 1) {
    if (attempt > 50) throw new Error("Herdr never took the stand-in's session");
    const seq = Date.now() * 1000 + attempt * 2;
    execFileSync(config.herdr, ["pane", "report-agent", pane, "--source", "herdr:opencode", "--agent", "opencode",
      "--state", "idle", "--agent-session-id", session, "--seq", String(seq)], { timeout: 10000 });
    execFileSync(config.herdr, ["pane", "report-agent-session", pane, "--source", "herdr:opencode", "--agent", "opencode",
      "--agent-session-id", session, "--session-start-source", "select", "--seq", String(seq + 1)], { timeout: 10000 });
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 100);
  }
}

const info = (id, parentID) => ({ id, slug: id, projectID: "prj_fixture", directory: process.cwd(), ...(parentID ? { parentID } : {}),
  title: "New session", version: "1.18.30", time: { created: Date.now(), updated: Date.now() } });
const sessions = { [session]: info(session) };
const running = {};
const client = { session: {
  get: async ({ path: { id } }) => ({ data: sessions[id] }),
  status: async () => ({ data: running }),
} };

let calls = 0;
const run = {
  async created(hooks, { id, parent }) {
    sessions[id] = info(id, parent);
    await hooks.event({ event: { type: "session.created", properties: { sessionID: id, info: sessions[id] } } });
    return {};
  },
  async status(hooks, { id, status }) {
    if (status === "busy") running[id] = { type: "busy" }; else delete running[id];
    await hooks.event({ event: { type: "session.status", properties: { sessionID: id, status: { type: status } } } });
    return {};
  },
  async prompt(hooks, { text }) {
    const messageID = "msg_fixture" + (calls += 1);
    const input = { sessionID: session, agent: "build", model: { providerID: "anthropic", modelID: "claude-sonnet" }, messageID };
    const output = { message: { id: messageID, sessionID: session, role: "user", time: { created: Date.now() } },
      parts: [{ id: "prt_0000000000000" + calls, sessionID: session, messageID, type: "text", text }] };
    await hooks["chat.message"](input, output);
    return { parts: output.parts };
  },
  async stored(hooks, { part }) {
    await hooks.event({ event: { type: "message.part.updated", properties: { sessionID: part.sessionID, part, time: Date.now() } } });
    return {};
  },
  async tool(hooks, { tool, args }) {
    try {
      await hooks["tool.execute.before"]({ tool, sessionID: session, callID: "call_" + (calls += 1) }, { args });
      return { ran: true };
    } catch (error) {
      return { refused: String(error && error.message) };
    }
  },
  async shell(hooks, { argv }) {
    try {
      return { stdout: execFileSync(argv[0], argv.slice(1), { encoding: "utf8", timeout: 20000 }) };
    } catch (error) {
      return { status: error.status, stdout: String(error.stdout || ""), stderr: String(error.stderr || "") };
    }
  },
};

(async () => {
  announce();
  // OpenCode 1.x runs the default definition's server.
  const { server } = (await import(pathToFileURL(config.plugin).href)).default;
  const hooks = await server({ client, directory: process.cwd() });
  await hooks.event({ event: { type: "session.created", properties: { sessionID: session, info: sessions[session] } } });
  fs.writeFileSync(path.join(folder, "ready.json.part"), JSON.stringify({ session, hooks: Object.keys(hooks) }));
  fs.renameSync(path.join(folder, "ready.json.part"), path.join(folder, "ready.json"));
  process.stdout.write("opencode stand-in ready\n");
  for (let next = 1; ; ) {
    const command = path.join(folder, "command-" + next + ".json");
    if (!fs.existsSync(command)) { await new Promise((resolve) => setTimeout(resolve, 25)); continue; }
    const request = JSON.parse(fs.readFileSync(command, "utf8"));
    const answer = await run[request.op](hooks, request);
    fs.writeFileSync(path.join(folder, "answer-" + next + ".json.part"), JSON.stringify(answer));
    fs.renameSync(path.join(folder, "answer-" + next + ".json.part"), path.join(folder, "answer-" + next + ".json"));
    next += 1;
  }
})().catch((error) => {
  fs.writeFileSync(path.join(folder, "failed.txt"), String(error && error.stack || error));
  process.exit(1);
});
`;

export type HostConfig = { herdr: string; stateDir: string; plugin: string; dir: string };

/** Writes the stand-in as `opencode` in `bin`, configured by `config`. */
export function installOpenCodeHost(bin: string, config: HostConfig): string {
  fs.writeFileSync(path.join(bin, "opencode-host.json"), JSON.stringify(config));
  return fixtureProgram(bin, "opencode", SCRIPT);
}

export type HostCommand =
  | { op: "created"; id: string; parent?: string }
  | { op: "status"; id: string; status: "busy" | "idle" }
  | { op: "prompt"; text: string }
  | { op: "stored"; part: Record<string, unknown> }
  | { op: "tool"; tool: string; args: Record<string, unknown> }
  | { op: "shell"; argv: string[] };

/** One running stand-in: its native session and a way to run a command in it. */
export type Host = { session: string; hooks: string[]; send: (command: HostCommand) => Promise<Record<string, unknown>> };

/** Waits for the stand-in in `pane` to have loaded the plugin, and returns its driver. */
export async function openCodeHost(dir: string, pane: string): Promise<Host> {
  const folder = path.join(dir, pane.replace(/[^A-Za-z0-9]/g, "-"));
  const ready = path.join(folder, "ready.json");
  await expect.poll(() => fs.existsSync(ready) || (fs.existsSync(path.join(folder, "failed.txt")) ? fs.readFileSync(path.join(folder, "failed.txt"), "utf8") : false),
    { message: `the OpenCode stand-in in ${pane} loaded the plugin`, timeout: 20_000 }).toBe(true);
  const { session, hooks } = JSON.parse(fs.readFileSync(ready, "utf8")) as { session: string; hooks: string[] };
  let sent = 0;
  return {
    session,
    hooks,
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
