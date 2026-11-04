import { expect } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { execFileSync } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { DatabaseSync } from "node:sqlite";
import { labelMarker, type HerdrFixture } from "./herdr-fixture";
import { copyFixtureShim } from "./shims/build";
import { fixtureExecutable, fixtureProgram } from "./platform-fixture";
import { agentIn, preparePaneInput, sendKeys, withControl } from "./daemon-actions";
import { builtHooks } from "./engine-fixture";
import type { E2eEngine } from "./e2e-engine";
import type { Daemon } from "./hided-fixture";

export const PI_ID = "fixture-pi-native";
export const PI_TITLE = "요청 보기 - Pi native title";
export const OMP_ID = "fixture-omp-native";
export const OMP_TITLE = "요청 보기 - omp native title";
export const OMP_UPDATED_TITLE = "요청 보기 - 현재 이름 日本語";
export const GROK_ID = "0199b000-0000-7000-8000-0000000000e2";
export const GROK_TITLE = "요청 보기 - Grok native title";
export const GROK_PLAN = "## 계획\n요청 보기를 두 단계로 나눈다";
export const CURSOR_ID = "0199b000-0000-7000-8000-0000000000c2";
export const CURSOR_GOAL = "요청 보기 - Cursor generated goal";
export type NativeFileReader = "pi" | "omp" | "grok" | "cursor";

/** Grok's group name for a cwd: urlencoding of every byte but `A-Za-z0-9-_.~`. */
function grokGroup(cwd: string): string {
  return [...Buffer.from(cwd)].map((byte) => /[A-Za-z0-9\-_.~]/.test(String.fromCharCode(byte)) ? String.fromCharCode(byte) : `%${byte.toString(16).toUpperCase().padStart(2, "0")}`).join("");
}

function grokRecord(update: Record<string, unknown>, meta: Record<string, unknown> = {}, extension = false): string {
  const at = 1_790_989_200_000;
  return `${JSON.stringify({ timestamp: at / 1000, method: extension ? "_x.ai/session/update" : "session/update", params: { sessionId: GROK_ID, update, _meta: { eventId: "fixture", agentTimestampMs: at, ...meta } } })}\n`;
}

/** Grok's plan approval as the pinned CLI records it: its `exit_plan_mode`
    call stays pending while `plan_mode.json` says the approval is awaited,
    with the plan itself in `plan.md`. Approval completes the call and clears the flag. */
export function setGrokPlanApproval(session: string, awaiting: boolean): void {
  const folder = path.dirname(session);
  if (awaiting) {
    fs.writeFileSync(path.join(folder, "plan.md"), `${GROK_PLAN}\n`);
    fs.appendFileSync(session, grokRecord({ sessionUpdate: "tool_call", toolCallId: "call-exit-plan", title: "exit_plan_mode", rawInput: {}, _meta: { "x.ai/tool": { version: 1, name: "exit_plan_mode", kind: "exit_plan" } } }, { promptId: "p-1" }));
  } else {
    fs.appendFileSync(session, grokRecord({ sessionUpdate: "tool_call_update", toolCallId: "call-exit-plan", status: "completed", title: "Plan mode exited", content: [{ type: "content", content: { type: "text", text: "Plan file: plan.md" } }] }, { promptId: "p-1" }));
  }
  const state = { state: awaiting ? "Active" : "Inactive", was_previously_active: true, reminder_count: 0, pending_exit_reminder: false, awaiting_plan_approval: awaiting };
  const temporary = path.join(folder, "plan_mode.json.tmp");
  fs.writeFileSync(temporary, JSON.stringify(state, null, 2));
  fs.renameSync(temporary, path.join(folder, "plan_mode.json"));
}

// Native omp's v1 title slot is exactly 256 bytes including its newline.
// This oracle comes from the pinned CLI format, independent of the reader.
function ompTitleSlot(title: string): Buffer {
  const record = { type: "title", v: 1, title, source: "user", updatedAt: "2026-10-03T01:00:00.500Z", pad: "" };
  const empty = JSON.stringify(record);
  const padding = 255 - Buffer.byteLength(empty);
  if (padding < 0) throw new Error("fixture title exceeds the native slot");
  return Buffer.from(`${JSON.stringify({ ...record, pad: " ".repeat(padding) })}\n`);
}

/** A native writer's in-place slot update, with all history bytes retained. */
export function updateOmpTitle(session: string, title: string): void {
  const file = fs.openSync(session, "r+");
  try {
    const slot = ompTitleSlot(title);
    if (fs.writeSync(file, slot, 0, slot.length, 0) !== slot.length) throw new Error("fixture title write was incomplete");
  } finally {
    fs.closeSync(file);
  }
}

/** Native recorded messages, not snapshot injection or question prose. */
export function appendOmpQuestion(session: string, answered = false): void {
  const timestamp = "2026-10-03T01:00:01Z";
  const message = answered
    ? { role: "toolResult", toolCallId: "omp-question", content: [{ type: "text", text: "미리보기" }], isError: true }
    : { role: "assistant", content: [{ type: "toolCall", id: "omp-question", name: "ask", arguments: { questions: [{ id: "target", question: "배포 대상을 골라주세요", options: [{ label: "미리보기" }, { label: "운영" }] }] } }], stopReason: "toolUse" };
  fs.appendFileSync(session, `${JSON.stringify({ type: "message", id: answered ? "answer" : "question", parentId: null, timestamp, message })}\n`);
}

/** What a native writer's seeds need of a run: its root and its HOME, which every engine's fixture has. */
type NativeWriterHost = { root: string; env: { HOME?: string } };

/**
 * A seed consumed and written by the fake CLI. No session file exists yet.
 * On the core's own engine the stand-in also reports the session it writes
 * as the agent's own integration does (`prepareEngineWriter`).
 */
export function prepareNativeWriter(herdr: NativeWriterHost, kind: NativeFileReader, checkout = path.join(herdr.root, "fixture")): string {
  copyFixtureShim("claude-shim", path.join(herdr.root, "bin", fixtureExecutable(kind === "cursor" ? "cursor-agent" : kind)));
  const cwd = fs.realpathSync(checkout);
  if (kind === "cursor") {
    // Ordinary pinned CLI routing, independently of the product locator.
    const bucket = createHash("md5").update(cwd).digest("hex");
    const session = path.join(herdr.env.HOME!, ".cursor", "chats", bucket, CURSOR_ID, "store.db");
    fs.mkdirSync(path.dirname(session), { recursive: true });
    fs.writeFileSync(path.join(herdr.root, "cursor-session-path.config"), session);
    fs.writeFileSync(path.join(herdr.root, "cursor-meta-seed.json"), JSON.stringify({ schemaVersion: 1, cwd, createdAtMs: 1_790_989_200_000, hasConversation: true, isSubagent: false }));
    const graph = JSON.parse(fs.readFileSync(new URL("../../hide-session/tests/fixtures/cursor-2026.10.01/browser-graph.json", import.meta.url), "utf8")) as { roots: { first: string }; blobs: Record<string, string> };
    const database = new DatabaseSync(path.join(herdr.root, "cursor-session-seed.db"));
    try {
      database.exec("PRAGMA user_version=1; CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT); CREATE TABLE blobs(id TEXT PRIMARY KEY,data BLOB);");
      const meta = { agentId: CURSOR_ID, latestRootBlobId: graph.roots.first, createdAt: 1_790_989_200_000 };
      database.prepare("INSERT INTO meta VALUES('0',?)").run(Buffer.from(JSON.stringify(meta)).toString("hex"));
      const insert = database.prepare("INSERT INTO blobs VALUES(?,?)");
      for (const [id, bytes] of Object.entries(graph.blobs)) insert.run(id, Buffer.from(bytes, "hex"));
    } finally {
      database.close();
    }
    return session;
  }
  const relativeTemp = path.relative(fs.realpathSync(os.tmpdir()), cwd);
  if (kind === "omp" && (relativeTemp.startsWith("..") || path.isAbsolute(relativeTemp))) throw new Error("omp fixture requires an owned temporary checkout");
  const encoded = kind === "pi" ? `--${cwd.replace(/^[/\\]/, "").replace(/[/\\:]/g, "-")}--` : `-tmp-${relativeTemp.replace(/[/\\:]/g, "-")}`;
  const session = kind === "grok"
    ? path.join(herdr.env.HOME!, ".grok", "sessions", grokGroup(cwd), GROK_ID, "updates.jsonl")
    : path.join(herdr.env.HOME!, `.${kind}`, "agent", "sessions", encoded, "timestamp_fixture.jsonl");
  fs.mkdirSync(path.dirname(session), { recursive: true });
  fs.writeFileSync(path.join(herdr.root, `${kind}-session-path.config`), session);
  if (kind === "grok") {
    fs.writeFileSync(path.join(herdr.root, "grok-summary-seed.json"), JSON.stringify({ info: { id: GROK_ID, cwd }, session_summary: GROK_TITLE, created_at: "2026-10-03T01:00:00Z", updated_at: "2026-10-03T01:00:00Z", num_messages: 3, current_model_id: "grok-build", generated_title: GROK_TITLE }, null, 2));
    fs.writeFileSync(path.join(herdr.root, "grok-session-seed.jsonl"), [
      grokRecord({ sessionUpdate: "user_message_chunk", content: { type: "text", text: "요청 보기를 만들어줘" }, _meta: { promptIndex: 0 } }),
      grokRecord({ sessionUpdate: "agent_message_chunk", content: { type: "text", text: labelMarker({task:"Generated Grok goal",progress:"완료"}) } }, { promptId: "p-0" }),
      grokRecord({ sessionUpdate: "turn_completed", prompt_id: "p-0", stop_reason: "end_turn" }, {}, true),
    ].join(""));
    return session;
  }
  const timestamp = "2026-10-03T01:00:00Z";
  const records = [
    { type: "session", version: 3, id: kind === "pi" ? PI_ID : OMP_ID, timestamp, cwd },
    { type: "message", id: "a0000001", parentId: null, timestamp, message: { role: "user", content: "요청 보기를 만들어줘" } },
    { type: "message", id: "a0000002", parentId: "a0000001", timestamp, message: { role: "assistant", content: [{ type: "text", text: labelMarker({task:"Generated Pi goal",progress:"완료"}) }], stopReason: "stop" } },
    ...(kind === "pi" ? [{ type: "session_info", id: "a0000003", parentId: "a0000002", timestamp, name: PI_TITLE }] : [{ type: "title_change", id: "a0000003", parentId: "a0000002", timestamp, title: "Old audit title", source: "user" }]),
  ];
  fs.writeFileSync(path.join(herdr.root, `${kind}-session-seed.jsonl`), Buffer.concat([
    kind === "omp" ? ompTitleSlot(OMP_TITLE) : Buffer.alloc(0),
    Buffer.from(records.map(record => `${JSON.stringify(record)}\n`).join("")),
  ]));
  return session;
}

/** The session a native writer's integration names: Grok's and Cursor's own id, Pi's and omp's session file. */
export function nativeSessionRef(kind: NativeFileReader, session: string): string {
  return kind === "grok" ? GROK_ID : kind === "cursor" ? CURSOR_ID : session;
}

/** The ID inside the native record, also shown in the sidebar, not its file reference. */
export function nativeSessionId(kind: NativeFileReader): string {
  return { pi: PI_ID, omp: OMP_ID, grok: GROK_ID, cursor: CURSOR_ID }[kind];
}

/**
 * Herdr's integration report of a native writer's session, which the stand-in
 * has no Hide integration to replace on Herdr's engine. On the core's own
 * engine the stand-in reports it itself (`prepareEngineWriter`).
 */
export function reportNativeWriter(herdr: HerdrFixture, kind: NativeFileReader, pane: string, session: string): void {
  const reference = kind === "pi" || kind === "omp" ? ["--agent-session-path", session] : ["--agent-session-id", nativeSessionRef(kind, session)];
  execFileSync(herdr.bin, ["pane", "report-agent-session", pane, "--source", `herdr:${kind}`, "--agent", kind, ...reference, "--seq", "2", "--session-start-source", "clear"], { env: herdr.env, timeout: 30_000 });
}

/** Pi's and omp's extension sends the version of the script it is; the helper answers no other. */
function piExtensionVersion(): number {
  const source = fs.readFileSync(path.resolve("..", "hide-agent-hooks", "src", "pi_extension.rs"), "utf8");
  const version = /pub const VERSION: u32 = (\d+);/.exec(source)?.[1];
  if (version === undefined) throw new Error("hide-agent-hooks/src/pi_extension.rs no longer declares `pub const VERSION: u32 = N;`");
  return Number(version);
}

const shellQuoted = (word: string): string => `'${word.replaceAll("'", `'\\''`)}'`;

/**
 * The report each agent's own Hide integration makes when its session starts
 * or switches, as that integration makes it: Grok's and Cursor's SessionStart
 * command hook reads the session its input names, and Pi's and omp's script
 * asks the helper's `session` operation with its own version. `reference` is
 * the id the integration names: Pi's and omp's is the session file.
 */
function sessionReport(kind: NativeFileReader, reference: string): { env: Record<string, string>; args: string[]; input: unknown } {
  switch (kind) {
    case "pi":
    case "omp":
      return { env: {}, args: [kind, "session"], input: { session_id: reference, version: piExtensionVersion() } };
    case "grok":
      return { env: { GROK_HOOK_EVENT: "session_start", GROK_SESSION_ID: reference }, args: ["hook", "--runtime", "grok", "--event", "SessionStart"], input: { sessionId: reference, hook_event_name: "SessionStart" } };
    case "cursor":
      return { env: { CURSOR_VERSION: "2026.10.01" }, args: ["hook", "--runtime", "cursor", "--event", "SessionStart"], input: { conversation_id: reference, hook_event_name: "sessionStart" } };
  }
}

/**
 * On the core's own engine nothing but the agent's Hide integration names a
 * pane's session, so the stand-in `kind` runs that integration's report from
 * inside its pane each time it writes or resumes its session
 * (`shims/claude-shim.c`, `report_session`): this file is the command line,
 * then its input. The commands are written for the shell the shim's `popen`
 * runs; no Windows lane runs a spec that uses them.
 */
function writeSessionReport(root: string, kind: NativeFileReader, session: string): void {
  if (process.platform === "win32") throw new Error("a native writer's session report is written for a Unix shell; no Windows lane runs it");
  const { env, args, input } = sessionReport(kind, nativeSessionRef(kind, session));
  const words = [...Object.entries(env).map(([name, value]) => `${name}=${shellQuoted(value)}`), shellQuoted(builtHooks()), ...args.map(shellQuoted)];
  let command = `${words.join(" ")} >/dev/null 2>&1`;
  if (kind === "omp") {
    // The seed ends with a completed assistant reply. Session identity alone
    // reports no activity, and omp has no screen rules. Its real extension
    // also reports that completed turn at agent_end, from inside its pane.
    const turn = JSON.stringify({ session_id: session, version: piExtensionVersion(), turn: "done" });
    command += ` && printf %s ${shellQuoted(turn)} | ${shellQuoted(builtHooks())} ${kind} turn >/dev/null 2>&1`;
  }
  fs.writeFileSync(path.join(root, `${kind}-session-report.config`), `${command}\n${JSON.stringify(input)}`);
}

/**
 * A native writer for the engine `fixture` runs on: the seeds and the session
 * path (`prepareNativeWriter`), and on the core's own engine the stand-in's
 * own session report. On Herdr the spec reports the session through Herdr's
 * integration (`reportNativeWriter`).
 */
export function prepareEngineWriter(fixture: { root: string; engine: E2eEngine }, kind: NativeFileReader, checkout?: string): string {
  const session = prepareNativeWriter({ root: fixture.root, env: { HOME: path.join(fixture.root, "home") } }, kind, checkout);
  if (fixture.engine === "own") writeSessionReport(fixture.root, kind, session);
  return session;
}

export const OPENCODE_ID = "ses_fixtureopencoderoot";
export const OPENCODE_TITLE = "요청 보기 - OpenCode native title";
export const OPENCODE_CHILD_TITLE = "Explore the code (@explore subagent)";

/**
 * OpenCode 1.18.30's own tables, as its migrations create them for these reads, and the write-ahead log and
 * busy wait it opens its database with, so a reader never blocks its writes.
 */
export const OPENCODE_SCHEMA = `
PRAGMA journal_mode = WAL;
PRAGMA busy_timeout = 5000;
CREATE TABLE IF NOT EXISTS session (id text PRIMARY KEY, project_id text NOT NULL, parent_id text, slug text NOT NULL,
  directory text NOT NULL, title text NOT NULL, version text NOT NULL, time_created integer NOT NULL, time_updated integer NOT NULL);
CREATE TABLE IF NOT EXISTS message (id text PRIMARY KEY, session_id text NOT NULL REFERENCES session(id) ON DELETE CASCADE,
  time_created integer NOT NULL, time_updated integer NOT NULL, data text NOT NULL);
CREATE TABLE IF NOT EXISTS part (id text PRIMARY KEY, message_id text NOT NULL REFERENCES message(id) ON DELETE CASCADE,
  session_id text NOT NULL, time_created integer NOT NULL, time_updated integer NOT NULL, data text NOT NULL);`;

// Run as `opencode` in a pane: a first launch writes its session into
// OpenCode's database the way OpenCode does (a root session in the pane's
// checkout and a subagent's child session under it), a `-s <id>` launch
// continues it without writing, and each reports its session id the way
// OpenCode's integration does, from this process, the pane's foreground
// program. On Herdr that is Herdr's OpenCode integration (`pane report-agent`
// and `report-agent-session`); on the core's own engine it is Hide's plugin
// asking the helper's `session` operation, and a line typed into the pane
// (`working`, `idle`) is the turn the plugin reports (`turn`). Every argument
// list is logged.
const OPENCODE_DOUBLE = String.raw`
const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");
// Node 22 calls its SQLite module experimental; the pane shows OpenCode, not that notice.
process.removeAllListeners("warning");
const { DatabaseSync } = require("node:sqlite");
const config = JSON.parse(fs.readFileSync(path.join(path.dirname(process.argv[1]), "opencode-double.json"), "utf8"));
const argv = process.argv.slice(2);
fs.appendFileSync(config.launches, JSON.stringify(argv) + "\n");
// A wake selects the session first and may add options after it, such as the default full-access option.
if (!(argv[0] === "-s" && argv[1] === config.id)) {
  fs.mkdirSync(path.dirname(config.database), { recursive: true });
  const db = new DatabaseSync(config.database);
  db.exec(config.schema);
  const at = Date.now();
  const session = db.prepare("INSERT INTO session VALUES (?, 'prj_fixture', ?, ?, ?, ?, '1.18.30', ?, ?)");
  const message = db.prepare("INSERT INTO message VALUES (?, ?, ?, ?, ?)");
  const part = db.prepare("INSERT INTO part VALUES (?, ?, ?, ?, ?, ?)");
  const directory = fs.realpathSync(process.cwd());
  session.run(config.id, null, "brave-otter", directory, config.title, at, at + 2);
  message.run("msg_01", config.id, at, at, JSON.stringify({ role: "user", time: { created: at } }));
  part.run("prt_01", "msg_01", config.id, at, at, JSON.stringify({ type: "text", text: "요청 보기를 만들어줘" }));
  message.run("msg_02", config.id, at + 1, at + 2, JSON.stringify({ role: "assistant", time: { created: at + 1, completed: at + 2 } }));
  part.run("prt_02", "msg_02", config.id, at + 1, at + 1, JSON.stringify({ type: "text", text: config.answer }));
  session.run(config.child, config.id, "quiet-heron", directory, config.childTitle, at + 1, at + 1);
  message.run("msg_c1", config.child, at + 1, at + 1, JSON.stringify({ role: "user", time: { created: at + 1 } }));
  part.run("prt_c1", "msg_c1", config.child, at + 1, at + 1, JSON.stringify({ type: "text", text: "subagent task" }));
  db.close();
}
// The helper answers on its standard output; the pane shows OpenCode, not that.
const helper = (operation, input) => {
  const answer = JSON.parse(execFileSync(config.hooks, ["opencode", operation], { input: JSON.stringify(input), encoding: "utf8", timeout: 10000 }));
  if (answer.reported !== true) throw new Error("OpenCode fixture's " + operation + " report was refused");
};
if (config.herdr) {
  const pane = process.env.HERDR_PANE_ID;
  const listed = () => JSON.parse(execFileSync(config.herdr, ["agent", "list"], { encoding: "utf8", timeout: 10000 }))
    .result.agents.find((agent) => agent.pane_id === pane)?.agent_session?.value;
  for (let attempt = 1; listed() !== config.id; attempt += 1) {
    if (attempt > 50) throw new Error("Herdr never took the double's session");
    const seq = Date.now() * 1000 + attempt * 2;
    execFileSync(config.herdr, ["pane", "report-agent", pane, "--source", "herdr:opencode", "--agent", "opencode",
      "--state", "idle", "--agent-session-id", config.id, "--seq", String(seq)], { timeout: 10000 });
    execFileSync(config.herdr, ["pane", "report-agent-session", pane, "--source", "herdr:opencode", "--agent", "opencode",
      "--agent-session-id", config.id, "--session-start-source", "select", "--seq", String(seq + 1)], { timeout: 10000 });
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 100);
  }
} else {
  helper("session", { session_id: config.id });
  require("node:readline").createInterface({ input: process.stdin }).on("line", (line) => {
    const [request, state] = line.trim().split(" ");
    const turn = { working: "working", idle: "done" }[state];
    if (turn) {
      // The real TUI redraws the interrupt hint as a turn starts and ends.
      // Its screen transition also gives the native reader a new engine state.
      process.stdout.write("\x1b[2J\x1b[H" + (state === "working" ? "opencode esc to interrupt\n" : "opencode fixture ready\n"));
      helper("turn", { session_id: config.id, turn });
      fs.writeFileSync(config.turnReceipt, request);
    }
  });
}
process.stdout.write("opencode fixture ready\n");
setInterval(() => {}, 1 << 30);
`;

/** What the OpenCode double needs of a run: the engine, its root (and HOME under it) and, on Herdr, the binary its integration reports through. */
type OpenCodeHost = { engine: E2eEngine; root: string; bin?: string };

/** Installs the OpenCode double as `opencode`; answers its database's path. */
export function prepareOpenCodeWriter(fixture: OpenCodeHost): string {
  const database = path.join(fixture.root, "home", ".local", "share", "opencode", "opencode.db");
  const bin = path.join(fixture.root, "bin");
  fs.writeFileSync(path.join(bin, "opencode-double.json"), JSON.stringify({
    herdr: fixture.engine === "herdr" ? fixture.bin : undefined, hooks: builtHooks(), database, schema: OPENCODE_SCHEMA, id: OPENCODE_ID, title: OPENCODE_TITLE,
    child: `${OPENCODE_ID}child`, childTitle: OPENCODE_CHILD_TITLE, launches: path.join(fixture.root, "opencode-launches.jsonl"),
    turnReceipt: path.join(fixture.root, "opencode-turn-receipt"),
    answer: labelMarker({task:"Generated OpenCode goal",progress:"완료"}),
  }));
  fixtureProgram(bin, "opencode", OPENCODE_DOUBLE);
  return database;
}

/** What OpenCode writes while its `question` tool waits, and once it is answered. */
export function writeOpenCodeQuestion(database: string, state: "running" | "completed"): void {
  const db = new DatabaseSync(database);
  db.exec(OPENCODE_SCHEMA);
  const at = Date.now();
  const input = { questions: [{ question: "어느 브랜치에 올릴까요?", header: "Branch", options: [{ label: "main", description: "기본" }, { label: "release", description: "배포" }] }] };
  const time = state === "running" ? { created: at } : { created: at - 1, completed: at };
  const toolState = state === "running" ? { status: "running", input, time: { start: at } } : { status: "completed", input, output: "main", title: "", metadata: {}, time: { start: at - 1, end: at } };
  db.prepare("INSERT INTO message VALUES ('msg_03', ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET data = excluded.data, time_updated = excluded.time_updated")
    .run(OPENCODE_ID, at, at, JSON.stringify({ role: "assistant", time }));
  db.prepare("INSERT INTO part VALUES ('prt_03', 'msg_03', ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET data = excluded.data, time_updated = excluded.time_updated")
    .run(OPENCODE_ID, at, at, JSON.stringify({ type: "tool", tool: "question", callID: "call_q1", state: toolState }));
  db.close();
}

let reported = 0;

/**
 * OpenCode's integration telling the engine the pane's state as OpenCode
 * moves, and waiting until the engine lists the pane so.
 * On Herdr that is its integration's `pane report-agent`, whose sequence is
 * time-based like the integration's and rises within one millisecond, since
 * Herdr keeps only a newer report. On the core's own engine it is the plugin's
 * `turn` report: the line typed into the pane is read by the double
 * (`OPENCODE_DOUBLE`), which asks the helper from inside the pane.
 */
export async function reportOpenCodeState(fixture: OpenCodeHost, daemon: Pick<Daemon, "origin" | "token" | "stateDir">, pane: string, state: "working" | "idle"): Promise<void> {
  if (fixture.engine === "own") {
    const request = randomUUID();
    const receipt = path.join(fixture.root, "opencode-turn-receipt");
    const engineRead = () => fs.readFileSync(path.join(daemon.stateDir, "Logs", "core.jsonl"), "utf8").trim().split("\n")
      .map(line => JSON.parse(line) as { kind?: string; pane_id?: string; state_change_seq?: number; activity?: string })
      .findLast(record => record.kind === "pane.read_record" && record.pane_id === pane);
    const before = engineRead()?.state_change_seq;
    expect(before, "the engine has read the initial OpenCode screen").toBeDefined();
    await withControl(daemon, async (control) => {
      await preparePaneInput(control, pane);
      sendKeys(control, pane, `${request} ${state}\r`);
      // A row already at rest cannot acknowledge a new idle report. Keep
      // the input connection until the double has reported this command.
      await expect.poll(() => fs.existsSync(receipt) ? fs.readFileSync(receipt, "utf8") : "",
        { message: `OpenCode reported ${state}`, timeout: 10_000 }).toBe(request);
      // A plugin report can change activity before the holder reads its new
      // screen. Do not draw the next state until that engine transition lands.
      await expect.poll(() => engineRead()?.state_change_seq,
        { message: `the engine read OpenCode's ${state} screen`, timeout: 10_000 }).toBeGreaterThan(before!);
      await control.until((rest) => {
        const row = agentIn(rest, pane);
        return row && (state === "working" ? row.activity === "working" : row.activity !== "working") ? true : undefined;
      }, `${pane} listed ${state}`, 10_000);
    });
    return;
  }
  const herdr = fixture as unknown as HerdrFixture;
  reported = Math.max(reported + 1, Date.now() * 1000);
  execFileSync(herdr.bin, ["pane", "report-agent", pane, "--source", "herdr:opencode", "--agent", "opencode", "--state", state,
    "--agent-session-id", OPENCODE_ID, "--seq", String(reported)], { env: herdr.env, timeout: 30_000 });
  type Listed = { result: { agents: { pane_id: string; agent_status: string }[] } };
  await expect.poll(() => (herdr.run(["agent", "list"]) as Listed).result.agents.find((agent) => agent.pane_id === pane)?.agent_status,
    { message: `Herdr lists ${pane} ${state}`, timeout: 10_000 }).toMatch(state === "working" ? /^working$/ : /^(idle|done)$/);
}
