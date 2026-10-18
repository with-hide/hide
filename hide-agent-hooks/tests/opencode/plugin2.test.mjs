// Hide's OpenCode 2 plugin, run in Node the way OpenCode 2.0.29 runs it: the server half and the terminal half the
// helper crate generates (`hide_agent_hooks::opencode::{SERVER, TUI}`), loaded from one folder as OpenCode loads
// `plugins/hide-opencode/`, on a stand-in for OpenCode's one shared server. That server runs a plugin instance per
// folder and routes a call to the instance of the folder it names, gives every instance every event, and each
// terminal attached to it is a separate pane. The stand-in helper records each call with the pane it ran in.
//
// `tests/it/opencode_plugin.rs` writes the halves and the stand-in and runs this file with HIDE_OPENCODE2_PLUGIN_DIR,
// HIDE_OPENCODE_PLUGIN (the 1.x file), HIDE_OPENCODE_HELPER_LOG and HIDE_OPENCODE_HELPER_ANSWERS set.

import assert from "node:assert/strict";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";
import { test } from "node:test";
import { pathToFileURL } from "node:url";

const DIR = process.env.HIDE_OPENCODE2_PLUGIN_DIR;
const { default: server } = await import(pathToFileURL(path.join(DIR, "server.js")).href);
const { default: tui } = await import(pathToFileURL(path.join(DIR, "tui.js")).href);
const LOG = process.env.HIDE_OPENCODE_HELPER_LOG;
const ANSWERS = process.env.HIDE_OPENCODE_HELPER_ANSWERS;

const PROJECT = "/checkouts/fixture";
// Where the first terminal started, so the folder OpenCode's calls go to when they name none.
const LAUNCH = "/checkouts";
const PANE_VARIABLES = ["HERDR_ENV", "HERDR_PANE_ID", "HERDR_SOCKET_PATH"];
const pane = (id) => ({ HERDR_ENV: "1", HERDR_PANE_ID: id, HERDR_SOCKET_PATH: "/tmp/herdr-fixture.sock" });
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** Writes the guardrails policy the core would publish, with `rules` on the fixture project. */
function policy(rules) {
  const file = path.join(homedir(), ".hide", "agent-hooks", "guardrails.json");
  mkdirSync(path.dirname(file), { recursive: true });
  const all = { worktree_folder: false, base_branch: false, agent_merge: false, git_commands: false, files: false, paths: [] };
  writeFileSync(file, JSON.stringify({ version: 1, projects: [{ root: PROJECT, base: null, rules: { ...all, ...rules } }] }));
}

policy({});

function answer(answers) {
  writeFileSync(ANSWERS, JSON.stringify(answers));
}

function calls(operation) {
  let lines = [];
  try {
    lines = readFileSync(LOG, "utf8").split("\n").filter(Boolean);
  } catch {}
  return lines.map((line) => JSON.parse(line)).filter((call) => !operation || call.operation === operation);
}

async function until(operation, count = 1, matches = () => true) {
  for (let waited = 0; waited < 5000; waited += 20) {
    const found = calls(operation).filter((call) => matches(call.input));
    if (found.length >= count) return found;
    await sleep(20);
  }
  assert.fail(`no ${operation} call: ${JSON.stringify(calls())}`);
}

/** Calls `subscriber` for each event published until it unsubscribes. */
class Bus {
  subscribers = new Set();
  on(subscriber) {
    this.subscribers.add(subscriber);
    return () => this.subscribers.delete(subscriber);
  }
  emit(event) {
    for (const subscriber of [...this.subscribers]) subscriber(event);
  }
}

/** OpenCode 2's shared server: sessions, the server half's instance per folder, and the events every client sees. */
class OpenCode {
  sessions = new Map();
  running = new Set();
  instances = new Map();
  events = new Bus();
  synthetic = [];
  asks = [];
  cleanups = [];

  /** The server half's instance for `directory`, started as OpenCode starts one. */
  async instance(directory) {
    const instance = { directory, hooks: {}, handlers: null };
    const ctx = (instance.ctx = {
      location: { directory },
      rpc: {
        register: async (definition, handlers) => {
          instance.handlers = handlers;
          return {
            events: {
              emit: async (name, data) => {
                this.asks.push(data);
                this.events.emit({ type: `rpc.${definition.id}.${name}`, data, location: { directory } });
              },
            },
          };
        },
      },
      session: {
        hook: async (name, hook) => {
          instance.hooks[name] = hook;
        },
        synthetic: async (input) => {
          const item = { id: `inb_${this.synthetic.length + 1}`, sessionID: input.sessionID, type: "synthetic", payload: input };
          this.synthetic.push(item);
          return item;
        },
        get: async ({ sessionID }) => this.sessions.get(sessionID),
      },
      tool: {
        hook: async (name, hook) => {
          instance.hooks[`tool.${name}`] = hook;
        },
      },
      event: { subscribe: ({ signal }) => this.subscribe(signal) },
    });
    this.cleanups.push(await server.setup(ctx));
    this.instances.set(directory, instance);
    return instance;
  }

  async *subscribe(signal) {
    const queue = [];
    let wake = () => {};
    const off = this.events.on((event) => {
      if (!event.type.startsWith("rpc.")) queue.push(event);
      wake();
    });
    signal.addEventListener("abort", () => wake());
    try {
      while (!signal.aborted) {
        if (queue.length > 0) yield queue.shift();
        else await new Promise((resolve) => (wake = resolve));
      }
    } finally {
      off();
    }
  }

  /** A terminal in Herdr pane `id` (or outside one, with no id), showing `route`. */
  terminal(id, route = { type: "home" }) {
    const view = { route, attached: [], gone: false };
    for (const name of PANE_VARIABLES) delete process.env[name];
    if (id) Object.assign(process.env, pane(id));
    const call = (method) => async (input, options) => {
      // A terminal that is gone sends nothing.
      if (view.gone) return {};
      if (method === "attach") view.attached.push({ ...input, location: options?.location });
      const instance = this.instances.get(options?.location?.directory ?? LAUNCH);
      if (!instance) throw { type: "RpcError", message: "no plugin at that location" };
      return await instance.handlers[method](input);
    };
    const api = {
      client: {
        rpc: (definition) => ({
          attach: call("attach"),
          detach: call("detach"),
          answer: call("answer"),
          events: {
            on: (name, handler) =>
              this.events.on((event) => {
                if (event.type === `rpc.${definition.id}.${name}`) void handler(event);
              }),
          },
        }),
      },
      ui: { router: { current: () => view.route } },
      data: {
        session: {
          get: (session) => this.sessions.get(session),
          root: (session) => {
            let current = session;
            while (this.sessions.get(current)?.parentID) current = this.sessions.get(current).parentID;
            return current;
          },
          status: (session) => (this.running.has(session) ? "running" : "idle"),
        },
        listen: (listener) => this.events.on((event) => listener({ details: event })),
      },
    };
    view.cleanup = tui.setup(api);
    this.cleanups.push(view.cleanup);
    for (const name of PANE_VARIABLES) delete process.env[name];
    return view;
  }

  /** Ends a terminal the way a killed process ends: it says nothing on its way out. */
  kill(view) {
    view.gone = true;
    view.cleanup?.();
  }

  /** A session OpenCode created in `directory`, announced to every client. */
  create(sessionID, parentID, directory = PROJECT) {
    this.sessions.set(sessionID, { id: sessionID, parentID, location: { directory } });
    this.publish("session.created", { sessionID, parentID, location: { directory } });
  }

  publish(type, data) {
    this.events.emit({ type, data, location: { directory: PROJECT } });
  }

  /** The operator's prompt in `sessionID`, through the instance of its folder; the synthetic items it added. */
  async prompt(sessionID, text = "fix the bug") {
    const before = this.synthetic.length;
    const instance = this.instances.get(this.sessions.get(sessionID).location.directory);
    await instance.hooks.prompt({ sessionID, messageID: "msg_1", prompt: { text, files: [] }, metadata: {}, delivery: "queue" });
    return this.synthetic.slice(before);
  }

  /** A tool call through the instance of the session's folder: its refusal, or null when it may run. */
  async tool(sessionID, tool, input) {
    const instance = this.instances.get(this.sessions.get(sessionID).location.directory);
    try {
      await instance.hooks["tool.execute.before"]({ tool, sessionID, agent: "build", messageID: "msg_1", id: "call_1", input });
      return null;
    } catch (error) {
      return error.message;
    }
  }

  /** A completed tool result through the instance of the session's folder, as the model would read it. */
  async result(sessionID, id = "call_1") {
    const instance = this.instances.get(this.sessions.get(sessionID).location.directory);
    const event = { tool: "shell", sessionID, id, status: "completed", result: { output: "RESULT", content: [{ type: "text", text: "RESULT" }], metadata: {} } };
    await instance.hooks["tool.execute.after"](event);
    return event.result.content;
  }

  close() {
    for (const cleanup of this.cleanups) cleanup?.();
  }
}

/** One shared server with an instance where the first terminal started and one in the project, as OpenCode runs it. */
async function opencode(t) {
  rmSync(LOG, { force: true });
  const host = new OpenCode();
  await host.instance(LAUNCH);
  await host.instance(PROJECT);
  t.after(() => host.close());
  return host;
}

/** Waits until the terminal attached to `root`. */
async function attached(view, root) {
  for (let waited = 0; waited < 2000; waited += 20) {
    if (view.attached.some((entry) => entry.root === root)) return;
    await sleep(20);
  }
  assert.fail(`never attached to ${root}: ${JSON.stringify(view.attached)}`);
}

test("OpenCode 2 reads the 1.x file as a plugin that does nothing", async () => {
  const { default: v1 } = await import(pathToFileURL(process.env.HIDE_OPENCODE_PLUGIN).href);
  assert.equal(typeof v1.id, "string");
  assert.equal(typeof v1.setup, "function");
  assert.equal(v1.setup({}), undefined);
  assert.equal(typeof v1.server, "function");
});

test("each terminal answers for the session it shows, in its own pane, whichever folder the session is in", async (t) => {
  answer({
    start: { context: "HIDE-GUIDANCE" },
    prompt: { context: "Hide letter letter-1 from lead (claude) [request]\nBODY-1\n", letters: ["letter-1"] },
    confirm: { confirmed: ["letter-1"] },
    subagents: { reported: true },
  });
  const host = await opencode(t);
  host.create("ses_a");
  host.create("ses_b");
  const a = host.terminal("w1:p1", { type: "session", sessionID: "ses_a" });
  const b = host.terminal("w1:p2", { type: "session", sessionID: "ses_b" });
  await attached(a, "ses_a");
  await attached(b, "ses_b");
  // Attached where the session's plugin runs: OpenCode sends a call that names no folder to the launch folder's.
  assert.deepEqual(a.attached[0].location, { directory: PROJECT });
  await until("start", 2);

  const [item] = await host.prompt("ses_b");
  const [call] = await until("prompt");
  assert.equal(call.env.HERDR_PANE_ID, "w1:p2");
  assert.equal(call.input.session_id, "ses_b");
  assert.equal(call.input.first, true);
  assert.equal(call.input.cwd, PROJECT);
  // Written ahead of the prompt as OpenCode's synthetic message, with Hide's own reminder tags.
  assert.equal(item.sessionID, "ses_b");
  assert.equal(item.payload.resume, false);
  assert.match(item.payload.text, /^<system-reminder>\nHIDE-GUIDANCE\n\nHide letter letter-1/);
  assert.match(item.payload.text, /\n<\/system-reminder>$/);

  // A letter is confirmed only once OpenCode delivered the message into the session.
  await sleep(100);
  assert.deepEqual(calls("confirm"), []);
  host.publish("session.inbox.delivered", { sessionID: "ses_b", inboxID: item.id });
  const [confirm] = await until("confirm");
  assert.deepEqual(confirm.input, { session_id: "ses_b", letters: ["letter-1"] });
  assert.equal(confirm.env.HERDR_PANE_ID, "w1:p2");

  // The guidance goes with a session's first prompt only.
  answer({ prompt: { context: "MEMORY" } });
  const [again] = await host.prompt("ses_b", "and the test");
  assert.doesNotMatch(again.payload.text, /HIDE-GUIDANCE/);
  await host.prompt("ses_a");
  const prompts = await until("prompt", 3);
  assert.deepEqual(
    prompts.map((call) => [call.env.HERDR_PANE_ID, call.input.session_id, call.input.first]),
    [
      ["w1:p2", "ses_b", true],
      ["w1:p2", "ses_b", false],
      ["w1:p1", "ses_a", true],
    ],
  );
});

test("a prompt in a session the terminal has just opened is answered before its route is read again", async (t) => {
  answer({ prompt: { context: "MEMORY" } });
  const host = await opencode(t);
  const view = host.terminal("w1:p1");
  host.create("ses_new");
  view.route = { type: "session", sessionID: "ses_new" };
  const [item] = await host.prompt("ses_new");
  assert.match(item?.payload.text ?? "", /MEMORY/);
  const [call] = await until("prompt");
  assert.equal(call.env.HERDR_PANE_ID, "w1:p1");
});

test("a letter due while the session works is written after a tool result and confirmed once OpenCode stored it", async (t) => {
  answer({
    letters: { context: "Hide letter letter-2 from lead (claude) [request]\nBODY-2\n", letters: ["letter-2"] },
    confirm: { confirmed: ["letter-2"] },
  });
  const host = await opencode(t);
  host.create("ses_a");
  const view = host.terminal("w1:p1", { type: "session", sessionID: "ses_a" });
  await attached(view, "ses_a");

  const content = await host.result("ses_a");
  assert.equal(content.length, 2);
  assert.equal(content[0].text, "RESULT");
  assert.match(content[1].text, /^<system-reminder>\nHide letter letter-2/);
  const [call] = await until("letters");
  assert.equal(call.env.HERDR_PANE_ID, "w1:p1");
  // While those letters wait for their receipt, the next result asks nothing.
  assert.equal((await host.result("ses_a", "call_2")).length, 1);
  assert.equal(calls("letters").length, 1);

  // Stored without Hide's text (a later plugin rewrote it), nothing is confirmed.
  host.publish("session.tool.success", { sessionID: "ses_a", id: "call_1", content: [{ type: "text", text: "RESULT" }] });
  await sleep(100);
  assert.deepEqual(calls("confirm"), []);

  const again = await host.result("ses_a", "call_3");
  host.publish("session.tool.success", { sessionID: "ses_a", id: "call_3", content: again });
  const [confirm] = await until("confirm");
  assert.deepEqual(confirm.input, { session_id: "ses_a", letters: ["letter-2"] });
});

test("the spawn and Factory guards run in the pane showing the session, a subagent's question as its root's", async (t) => {
  answer({ tool: { deny: "Not run: use hide agent spawn" } });
  const host = await opencode(t);
  host.create("ses_a");
  host.create("ses_child", "ses_a");
  const view = host.terminal("w1:p1", { type: "session", sessionID: "ses_a" });
  await attached(view, "ses_a");

  assert.equal(await host.tool("ses_a", "shell", { command: "herdr agent start --kind claude", workdir: "/w" }), "Not run: use hide agent spawn");
  const [launch] = await until("tool");
  assert.deepEqual(launch.input, { tool: "shell", command: "herdr agent start --kind claude", cwd: "/w", session_id: "ses_a" });
  assert.equal(launch.env.HERDR_PANE_ID, "w1:p1");
  // OpenCode runs a relative workdir in the session's folder, never where the shared server started.
  await host.tool("ses_a", "shell", { command: "herdr agent start --kind claude", workdir: "sub" });
  const [, relative] = await until("tool", 2);
  assert.equal(relative.input.cwd, path.join(PROJECT, "sub"));

  assert.equal(await host.tool("ses_child", "question", { questions: [] }), "Not run: use hide agent spawn");
  const [, , question] = await until("tool", 3);
  assert.deepEqual(question.input, { tool: "question", session_id: "ses_a" });

  // With no protection on, any other call runs without asking.
  assert.equal(await host.tool("ses_a", "shell", { command: "cargo test" }), null);
  assert.equal(await host.tool("ses_a", "read", { path: ".env" }), null);
  await sleep(100);
  assert.equal(calls("tool").length, 3);
});

test("a subagent whose line OpenCode cannot tell in time is asked of the terminal showing its root", async (t) => {
  answer({ tool: { deny: "Not run: use hide agent spawn" } });
  const host = await opencode(t);
  host.create("ses_a");
  const view = host.terminal("w1:p1", { type: "session", sessionID: "ses_a" });
  await attached(view, "ses_a");
  // Created before this instance started, and its lookup never answers: the server half cannot place it.
  host.sessions.set("ses_late", { id: "ses_late", parentID: "ses_a", location: { directory: PROJECT } });
  const lookups = host.instances.get(PROJECT);
  const get = lookups.ctx.session.get;
  lookups.ctx.session.get = async ({ sessionID }) => (sessionID === "ses_late" ? new Promise(() => {}) : get({ sessionID }));
  assert.equal(await host.tool("ses_late", "question", { questions: [] }), "Not run: use hide agent spawn");
  const [question] = await until("tool");
  assert.equal(question.env.HERDR_PANE_ID, "w1:p1");
  assert.deepEqual(question.input, { tool: "question", session_id: "ses_late" });
});

test("with no terminal for the session the server half only guards a protected project, outside any pane", async (t) => {
  answer({ prompt: { context: "MEMORY" }, tool: { deny: "Not run: guarded" } });
  const host = await opencode(t);
  host.create("ses_a");
  // `opencode run` and a terminal outside Herdr attach nothing.
  host.terminal(undefined, { type: "session", sessionID: "ses_a" });
  assert.deepEqual(await host.prompt("ses_a"), []);
  assert.equal(await host.tool("ses_a", "shell", { command: "herdr agent start --kind claude" }), null);
  await sleep(100);
  assert.deepEqual(calls(), []);

  policy({ files: true, paths: [".env"] });
  t.after(() => policy({}));
  assert.equal(await host.tool("ses_a", "patch", { patchText: "*** Begin Patch\n*** Update File: .env\n+x\n*** End Patch" }), "Not run: guarded");
  const [call] = await until("tool");
  assert.deepEqual(call.input, {
    tool: "patch",
    args: { patchText: "*** Begin Patch\n*** Update File: .env\n*** End Patch" },
    cwd: PROJECT,
    session_id: "ses_a",
    guard_only: true,
  });
  assert.equal(call.env.HERDR_PANE_ID, undefined);
});

test("a terminal that misses an answer is still asked, and the miss reaches the diagnostic", async (t) => {
  answer({ tool: { deny: "Not run: use hide agent spawn" } });
  const host = await opencode(t);
  host.create("ses_a");
  const view = host.terminal("w1:p1", { type: "session", sessionID: "ses_a" });
  await attached(view, "ses_a");
  const launch = { command: "herdr agent start --kind claude" };

  // Its answer is lost: the call runs, as a timed-out helper's does in the 1.x plugin.
  view.gone = true;
  assert.equal(await host.tool("ses_a", "shell", launch), null);
  view.gone = false;
  assert.equal(await host.tool("ses_a", "shell", launch), "Not run: use hide agent spawn");
  const tools = await until("tool", 2);
  assert.equal(tools[1].env.HERDR_PANE_ID, "w1:p1");
  assert.equal(tools[1].input.lost, 1);
});

test("a terminal that went away without a word is let go once it stops attaching, and the guardrails still hold", async (t) => {
  answer({ tool: { deny: "Not run: guarded" } });
  const host = await opencode(t);
  host.create("ses_a");
  const view = host.terminal("w1:p1", { type: "session", sessionID: "ses_a" });
  await attached(view, "ses_a");
  policy({ files: true, paths: [".env"], worktree_folder: false });
  t.after(() => policy({}));
  host.kill(view);
  t.mock.timers.enable({ apis: ["Date"], now: Date.now() });

  // Still attached: the question goes unanswered for its budget, then the guardrails are judged outside any pane.
  assert.equal(await host.tool("ses_a", "read", { path: ".env" }), "Not run: guarded");
  assert.equal(host.asks.length, 1);
  // Two renewals missed: it is asked no more.
  t.mock.timers.setTime(Date.now() + 10_001);
  assert.equal(await host.tool("ses_a", "read", { path: ".env", workdir: "x" }), "Not run: guarded");
  assert.equal(host.asks.length, 1);
  const tools = calls("tool").filter((call) => call.input.guard_only);
  assert.equal(tools.length, 2);
  for (const call of tools) assert.equal(call.env.HERDR_PANE_ID, undefined);
});

test("a pane that left a session answers for it no more, and a renewal never takes a session from the terminal that opened it", async (t) => {
  answer({ prompt: { context: "MEMORY" } });
  const host = await opencode(t);
  host.create("ses_a");
  const a = host.terminal("w1:p1", { type: "session", sessionID: "ses_a" });
  await attached(a, "ses_a");
  a.route = { type: "home" };
  await sleep(400);
  const b = host.terminal("w1:p2", { type: "session", sessionID: "ses_a" });
  await attached(b, "ses_a");
  // The first terminal renewing, as a terminal still showing the session does every few seconds.
  await host.instances.get(PROJECT).handlers.attach({ root: "ses_a", client: a.attached[0].client, renew: true });

  const [item] = await host.prompt("ses_a");
  assert.match(item.payload.text, /MEMORY/);
  b.route = { type: "home" };
  await sleep(400);
  // No terminal shows it now: nothing answers, the home screen's pane included.
  assert.deepEqual(await host.prompt("ses_a"), []);
  const prompts = calls("prompt");
  assert.deepEqual(prompts.map((call) => call.env.HERDR_PANE_ID), ["w1:p2"]);
});

test("a letter Hide is confirming is not written again by the next prompt", async (t) => {
  const letter = "Hide letter letter-1 from lead (claude) [request]\nBODY-1\n";
  answer({
    prompt: { context: `MEMORY\n${letter}`, letters: ["letter-1"] },
    confirm: { confirmed: ["letter-1"], delay_ms: 400 },
  });
  const host = await opencode(t);
  host.create("ses_a");
  const view = host.terminal("w1:p1", { type: "session", sessionID: "ses_a" });
  await attached(view, "ses_a");
  const [first] = await host.prompt("ses_a");
  host.publish("session.inbox.delivered", { sessionID: "ses_a", inboxID: first.id });
  await until("confirm");
  // The pull this prompt makes answers before the confirmation lands, still handing the letter over.
  await host.prompt("ses_a", "and the test");
  const written = host.synthetic.filter((item) => item.payload.text.includes("letter-1"));
  assert.equal(written.length, 1, JSON.stringify(host.synthetic.map((item) => item.payload.text)));
});

test("a subagent's run moves the counts of the pane showing its root, and the root's rest reports them again", async (t) => {
  answer({ subagents: { reported: true } });
  const host = await opencode(t);
  host.create("ses_a");
  const view = host.terminal("w1:p1", { type: "session", sessionID: "ses_a" });
  await attached(view, "ses_a");
  host.terminal("w1:p2", { type: "session", sessionID: "ses_other" });

  host.create("ses_child", "ses_a");
  host.running.add("ses_child");
  host.publish("session.execution.started", { sessionID: "ses_child" });
  await until("subagents", 1, (input) => input.working === 1);
  host.running.delete("ses_child");
  host.publish("session.execution.succeeded", { sessionID: "ses_child" });
  await until("subagents", 1, (input) => input.working === 0 && input.done === 1);
  const before = calls("subagents").length;
  host.publish("session.execution.succeeded", { sessionID: "ses_a" });
  await until("subagents", before + 1);
  // Only the pane showing the child's root counted it.
  const counted = calls("subagents").filter((call) => call.input.working > 0 || call.input.done > 0);
  assert.ok(counted.every((call) => call.env.HERDR_PANE_ID === "w1:p1"), JSON.stringify(counted));
});
