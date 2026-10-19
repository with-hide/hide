// Hide's OpenCode 2 plugin, its server half, written by Hide's install kit (hide-agent-hooks).
// An edit is kept and shown as edited in Settings; Reinstall puts Hide's back.
// OpenCode 2 runs one server for every terminal attached to it, so this half is never any pane's: what needs the
// pane (letters, guidance, Memory, the spawn guard, subagent counts) is asked of the TUI half, `tui.js`
// beside it, which runs in the terminal that shows the session. Without one it only guards a protected project.

import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { readFileSync, statSync } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";

const HELPER = __HIDE_HELPER__;
const ROUTE = __HIDE_ROUTE__;

// The budgets are the 1.x plugin's (`../hide.js`): the prompt one is Claude Code's prompt hook's, the tool one the
// spawn guard's. A question to the TUI half spends the same budget, its trip included.
export const PROMPT_BUDGET_MS = 1850;
export const TOOL_BUDGET_MS = 2500;
export const START_BUDGET_MS = 8000;
export const CONFIRM_BUDGET_MS = 2000;
export const COUNT_BUDGET_MS = 2000;
export const LETTERS_BUDGET_MS = 1500;
// What the TUI half keeps of a question's budget for the answer's way back.
export const ANSWER_MARGIN_MS = 150;
// A terminal attaches again this often while it shows a session, and one that has not for two of those is gone (a
// killed terminal says nothing on its way out); a slow answer alone never detaches it.
export const ATTACH_RENEW_MS = 5_000;
const ATTACH_LEASE_MS = 2 * ATTACH_RENEW_MS;
const LOOKUP_BUDGET_MS = 300;
const OUTPUT_LIMIT = 64 * 1024;
// Far inside the helper's 256 KiB input, so a pasted log never costs a prompt its letters.
const PROMPT_TEXT_LIMIT = 32 * 1024;
const RUNNING_LIMIT = 8;
export const SESSION_LIMIT = 512;
const PENDING_LIMIT = 16;
const PENDING_AGE_MS = 10 * 60 * 1000;
// Any spelling of the reminder tag OpenCode's model reads as one, opening or closing.
const REMINDER_TAG = /<(\s*\/?\s*system-reminder)/gi;
// The variables that make a process a Herdr pane's: the shared server inherited them from whichever terminal
// started it, so a helper this half starts never carries them.
const PANE_VARIABLES = ["HERDR_ENV", "HERDR_PANE_ID", "HERDR_SOCKET_PATH", "HIDE_CAP_REF"];

// The question and answer between the halves, on OpenCode's plugin RPC. The server half asks one terminal by the
// id it attached with (`client`), or, for a prompt in a session no terminal attached yet, any terminal that shows
// the session's root; the first answer is taken.
const OBJECT = { type: "object" };
export const RPC = {
  id: "hide.opencode",
  methods: {
    attach: { input: OBJECT, output: OBJECT },
    detach: { input: OBJECT, output: OBJECT },
    answer: { input: OBJECT, output: OBJECT },
  },
  events: { ask: { schema: OBJECT } },
};

// The project guardrails' policy, which Hide's core writes for this machine (`guardrails::policy_path`). A tool
// call reaches the helper for them only while some project has a protection on, read again when the file changes,
// so a machine with none starts no helper for a file tool (PRD agent-guardrails B27).
const POLICY = path.join(homedir(), ".hide", "agent-hooks", "guardrails.json");
const FILE_TOOLS = new Set(["read", "edit", "write", "patch"]);
// The arguments that name a file; the rest of a write (its content) is not sent.
const PATH_ARGS = ["path", "filePath"];
const policy = { mtimeMs: -1, size: -1, on: null };

/** Whether some project has a protection on: null when the policy is missing or unreadable. */
export function policyOn() {
  let stat;
  try {
    stat = statSync(POLICY);
  } catch {
    return null;
  }
  if (stat.mtimeMs === policy.mtimeMs && stat.size === policy.size) return policy.on;
  let on = null;
  try {
    const read = JSON.parse(readFileSync(POLICY, "utf8"));
    if (read?.version === 1 && Array.isArray(read.projects)) {
      on = read.projects.some((project) => {
        const rules = project?.rules;
        return Boolean(rules?.worktree_folder || rules?.base_branch || rules?.git_commands || rules?.files);
      });
    }
  } catch {}
  Object.assign(policy, { mtimeMs: stat.mtimeMs, size: stat.size, on });
  return on;
}

/** A file tool's arguments that name its files: its paths, and a patch's file lines. */
function fileArgs(args) {
  const named = {};
  for (const key of PATH_ARGS) if (typeof args?.[key] === "string") named[key] = args[key];
  if (typeof args?.patchText === "string") {
    named.patchText = args.patchText.split("\n").filter((line) => line.startsWith("*** ")).join("\n");
  }
  return named;
}

export function helperPresent() {
  try {
    return statSync(HELPER).isFile();
  } catch {
    return false;
  }
}

/** Whether `env` is a Herdr pane Hide manages, with the helper in place. */
export function managed(env) {
  return env.HERDR_ENV === "1" && Boolean(env.HERDR_PANE_ID) && Boolean(env.HERDR_SOCKET_PATH) && helperPresent();
}

/**
 * The helper's answer, or null when it failed, timed out or said nothing usable. `env` is the environment it runs
 * in: the TUI half's own, a pane's, or this half's without the pane variables it inherited.
 */
export function helper(state, operation, input, budgetMs, env = process.env) {
  // Once OpenCode disposed of the plugin, a report still waiting starts nothing.
  if (state.closed || budgetMs <= 0) return Promise.resolve(null);
  if (state.running.size >= RUNNING_LIMIT) {
    state.lost += 1;
    return Promise.resolve(null);
  }
  // A call skipped at the cap or given up on reaches Hide's diagnostic through the next one.
  if (state.lost > 0) {
    input = { ...input, lost: (Number(input.lost) || 0) + state.lost };
    state.lost = 0;
  }
  return new Promise((resolve) => {
    let child;
    try {
      child = spawn(HELPER, ["opencode", operation, ...ROUTE], { stdio: ["pipe", "pipe", "ignore"], env });
    } catch {
      resolve(null);
      return;
    }
    state.running.add(child);
    const chunks = [];
    let size = 0;
    let settled = false;
    const finish = (value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      state.running.delete(child);
      resolve(value);
    };
    const timer = setTimeout(() => {
      child.kill("SIGKILL");
      state.lost += 1;
      finish(null);
    }, budgetMs);
    child.on("error", () => finish(null));
    child.stdout.on("data", (chunk) => {
      size += chunk.length;
      if (size > OUTPUT_LIMIT) {
        child.kill("SIGKILL");
        finish(null);
        return;
      }
      chunks.push(chunk);
    });
    child.on("close", () => {
      try {
        finish(JSON.parse(Buffer.concat(chunks).toString("utf8")));
      } catch {
        finish(null);
      }
    });
    child.stdin.on("error", () => {});
    child.stdin.end(JSON.stringify(input));
  });
}

/** Ends every helper still running; nothing starts after. */
export function close(state) {
  state.closed = true;
  for (const child of state.running) child.kill("SIGKILL");
  state.running.clear();
}

export function withBudget(promise, budgetMs) {
  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve(null), budgetMs);
    Promise.resolve(promise).then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      () => {
        clearTimeout(timer);
        resolve(null);
      },
    );
  });
}

/**
 * Records `key` as the newest entry of `map`, a Map or a Set of at most SESSION_LIMIT entries. At the limit the
 * oldest entry the first matching rule of `evictable` accepts goes; when none may go the new one is not recorded
 * and the loss reaches the diagnostic through the next helper call.
 */
export function remember(state, map, key, value, evictable = [() => true]) {
  map.delete(key);
  if (map.size >= SESSION_LIMIT) {
    const entries = [...map.entries()];
    const oldest = evictable.map((rule) => entries.find(([name, entry]) => rule(entry, name))).find(Boolean);
    if (!oldest) {
      state.lost += 1;
      return;
    }
    map.delete(oldest[0]);
  }
  if (map instanceof Set) map.add(key);
  else map.set(key, value);
}

/** The start of a prompt, enough for Memory's retrieval, cut where JSON can still carry it. */
function clip(text) {
  // A lone surrogate anywhere would make the JSON the helper reads invalid.
  if (typeof text.toWellFormed === "function") text = text.toWellFormed();
  if (text.length <= PROMPT_TEXT_LIMIT) return text;
  const end = PROMPT_TEXT_LIMIT - (/[\uD800-\uDBFF]/.test(text[PROMPT_TEXT_LIMIT - 1]) ? 1 : 0);
  return text.slice(0, end);
}

/** Hide's text for the model: Hide writes the reminder tags itself, so a letter or Memory line cannot close them. */
function reminder(context) {
  return `<system-reminder>\n${context.replace(REMINDER_TAG, "<​$1")}\n</system-reminder>`;
}

function letterIds(answer) {
  return Array.isArray(answer?.letters) ? answer.letters.filter((letter) => typeof letter === "string") : [];
}

/** A shell call's folder: OpenCode runs a relative `workdir` in the session's folder, never the server's own. */
function workingFolder(workdir, directory) {
  if (typeof workdir !== "string" || !workdir) return directory;
  return typeof directory === "string" ? path.resolve(directory, workdir) : workdir;
}

/** The environment of a helper this half starts: the server's, without a pane it is not. */
function serverEnv() {
  const env = { ...process.env };
  for (const name of PANE_VARIABLES) delete env[name];
  return env;
}

/** The session above `sessionID` (null for a root), from OpenCode's events, else asked once; undefined when unknown. */
async function parentOf(state, ctx, sessionID) {
  if (state.parents.has(sessionID)) return state.parents.get(sessionID);
  let info;
  try {
    info = await withBudget(ctx.session.get({ sessionID }), LOOKUP_BUDGET_MS);
  } catch {
    return undefined;
  }
  info = info?.data ?? info;
  if (!info || info.id !== sessionID) return undefined;
  remember(state, state.parents, sessionID, info.parentID ?? null);
  return info.parentID ?? null;
}

/** The root session above `sessionID`, or undefined when its line is not known. */
async function rootOf(state, ctx, sessionID) {
  let current = sessionID;
  for (let depth = 0; depth < SESSION_LIMIT; depth += 1) {
    const parent = await parentOf(state, ctx, current);
    if (parent === undefined) return undefined;
    if (parent === null) return current;
    current = parent;
  }
  return undefined;
}

/** The terminal attached to `root` while its lease holds, or null; one past its lease is forgotten. */
function terminalOf(state, root) {
  const entry = state.attached.get(root);
  if (!entry) return null;
  if (Date.now() - entry.at > ATTACH_LEASE_MS) {
    state.attached.delete(root);
    return null;
  }
  return entry.client;
}

function attachTerminal(state, root, client) {
  remember(state, state.attached, root, { client, at: Date.now() });
}

/**
 * Asks the terminal attached to `root` (or, with `anyone`, any terminal showing it) and waits at most `budgetMs`
 * for its answer: the answer's output, or null when none came. A question the attached terminal left unanswered
 * reaches Hide's diagnostic through the next helper call; the terminal stays attached while it renews.
 */
function ask(state, registration, root, operation, input, budgetMs, anyone = false) {
  const client = terminalOf(state, root);
  if (!client && !anyone) return Promise.resolve(null);
  if (state.lost > 0) {
    input = { ...input, lost: state.lost };
    state.lost = 0;
  }
  const id = randomUUID();
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      state.questions.delete(id);
      // Only an attached terminal was expected to answer: a session no terminal shows (`opencode run`) has none.
      if (client) state.lost += 1;
      resolve(null);
    }, budgetMs);
    // Null when OpenCode disposed of the plugin before the answer came.
    state.questions.set(id, (answer) => {
      clearTimeout(timer);
      if (answer === null) {
        resolve(null);
        return;
      }
      if (typeof answer.client === "string" && answer.client) attachTerminal(state, root, answer.client);
      resolve(answer.output && typeof answer.output === "object" ? answer.output : {});
    });
    registration.events.emit("ask", { id, client, root, operation, input, budget_ms: budgetMs }).catch(() => {});
  });
}

/** Forgets the writes that never came; the letters they carried stay pending in Hide and ride the next ask. */
function forgetStale(state, now) {
  for (const [name, old] of state.receipts) {
    if (now - old.at > PENDING_AGE_MS) state.receipts.delete(name);
  }
}

/** Whether letters handed over in `root` still wait for OpenCode's write or Hide's confirmation. */
function handoverOpen(state, root) {
  forgetStale(state, Date.now());
  if (state.confirming.has(root)) return true;
  for (const entry of state.receipts.values()) {
    if (entry.root === root) return true;
  }
  return false;
}

/** Waits for OpenCode to write `key` with the letters; a letter left out stays pending in Hide. */
function holdReceipt(state, key, entry) {
  const now = Date.now();
  forgetStale(state, now);
  if (state.receipts.size >= PENDING_LIMIT) state.receipts.delete(state.receipts.keys().next().value);
  state.receipts.set(key, { ...entry, at: now });
}

/**
 * Confirms the letters OpenCode wrote; a letter Hide did not take stays pending there and rides the next ask. The
 * promise settles once Hide answered, or the confirmation's budget ran out.
 */
function confirm(state, registration, receipt) {
  const input = { session_id: receipt.root, letters: receipt.letters };
  const confirming = ask(state, registration, receipt.root, "confirm", input, CONFIRM_BUDGET_MS).then((answer) => {
    if (state.confirming.get(receipt.root) === confirming) state.confirming.delete(receipt.root);
    const confirmed = Array.isArray(answer?.confirmed) ? answer.confirmed : [];
    for (const letter of receipt.letters) if (confirmed.includes(letter)) remember(state, state.confirmed, letter);
  });
  state.confirming.set(receipt.root, confirming);
}

function toolKey(sessionID, callID) {
  return typeof sessionID === "string" && typeof callID === "string" ? `${sessionID}\u0000${callID}` : undefined;
}

/**
 * A root session's prompt: the context the terminal showing it gives (the start guidance and Memory on the
 * session's first prompt, Memory and the letters due), written as a synthetic message OpenCode admits ahead of the
 * prompt. The letters are confirmed once OpenCode delivered that message into the session; a confirmation still on
 * its way is waited for first, so the letters it takes are not handed over again.
 */
async function onPrompt(state, ctx, registration, event) {
  const deadline = Date.now() + PROMPT_BUDGET_MS;
  const sessionID = event?.sessionID;
  const text = event?.prompt?.text;
  if (typeof sessionID !== "string" || typeof text !== "string") return;
  // A subagent's prompt is no one's turn.
  if ((await parentOf(state, ctx, sessionID)) !== null) return;
  if (state.confirming.has(sessionID)) await withBudget(state.confirming.get(sessionID), Math.max(0, deadline - Date.now()));
  const answer = await ask(
    state,
    registration,
    sessionID,
    "prompt",
    { session_id: sessionID, prompt: clip(text), cwd: ctx.location?.directory },
    Math.max(0, deadline - Date.now()),
    true,
  );
  if (!answer) return;
  let letters = letterIds(answer);
  const sections = [answer.guidance];
  // A letter Hide already took is never written twice: the prompt then carries the guidance alone, and the letters
  // that came with it stay pending in Hide for the next ask.
  if (letters.some((letter) => state.confirmed.has(letter))) letters = [];
  else sections.push(answer.context);
  const context = sections.filter((section) => typeof section === "string" && section).join("\n\n");
  if (!context) return;
  const admitted = await ctx.session.synthetic({
    sessionID,
    text: reminder(context),
    description: "Hide",
    metadata: { source: "hide" },
    delivery: event.delivery,
    resume: false,
  });
  const id = (admitted?.data ?? admitted)?.id;
  if (letters.length === 0 || typeof id !== "string") return;
  if (state.delivered.has(id)) confirm(state, registration, { root: sessionID, letters });
  else holdReceipt(state, id, { root: sessionID, letters });
}

/**
 * The guards for one tool call: the terminal showing the session runs every one (the project's guardrails, the
 * spawn guard); with none, or none answering, this half runs the guardrails
 * alone, outside any pane. The refusal's reason, or null.
 */
async function onTool(state, ctx, registration, event) {
  const tool = event?.tool;
  const input = event?.input;
  const directory = ctx.location?.directory;
  let request;
  if (tool === "shell") {
    const command = input?.command;
    if (typeof command !== "string") return null;
    request = { tool, command, cwd: workingFolder(input.workdir, directory) };
  } else if (FILE_TOOLS.has(tool)) {
    request = { tool, args: fileArgs(input), cwd: directory };
  } else {
    return null;
  }
  // A session whose line OpenCode did not tell in time is taken as its own root, as the 1.x plugin takes it; any
  // terminal showing that session's root is then asked.
  const known = await rootOf(state, ctx, event.sessionID);
  const root = known ?? event.sessionID;
  const guarded = policyOn();
  // A shell call that can start an agent through Herdr is the spawn guard's.
  const launch = tool === "shell" && (request.command.includes("herdr") || request.command.includes("HERDR_BIN_PATH"));
  // The terminal is asked for what only a pane can judge, and then judges the guardrails too; a missing or
  // unreadable policy is judged there, where the helper records why.
  if ((known === undefined || terminalOf(state, root)) && (launch || guarded !== false)) {
    // The core knows the pane by the root session Herdr's integration reports, also for a subagent's call.
    const session_id = event.sessionID;
    const answer = await ask(state, registration, root, "tool", { ...request, session_id }, TOOL_BUDGET_MS, known === undefined);
    if (answer !== null) return typeof answer.deny === "string" && answer.deny ? answer.deny : null;
  }
  if (!guarded) return null;
  const answer = await helper(state, "tool", { ...request, session_id: event.sessionID, guard_only: true }, TOOL_BUDGET_MS, serverEnv());
  return typeof answer?.deny === "string" && answer.deny ? answer.deny : null;
}

/**
 * The letters due while the root session works, written after a tool's result so the agent reads them before its
 * next step (PRD letters-to-busy-agents D-02), and confirmed once OpenCode stored that result with them. A root
 * whose letters wait for their receipt asks nothing: those are not written twice.
 */
async function onToolResult(state, ctx, registration, event) {
  await observeTool(state, ctx, registration, event);
  const sessionID = event?.sessionID;
  const key = toolKey(sessionID, event?.id);
  if (!key || event.status !== "completed" || !Array.isArray(event.result?.content)) return;
  if (!terminalOf(state, sessionID) || (await parentOf(state, ctx, sessionID)) !== null) return;
  if (handoverOpen(state, sessionID)) return;
  const answer = await ask(state, registration, sessionID, "letters", { session_id: sessionID }, LETTERS_BUDGET_MS);
  if (typeof answer?.context !== "string" || !answer.context) return;
  const letters = letterIds(answer);
  // A letter Hide already took is never written twice.
  if (letters.some((letter) => state.confirmed.has(letter))) return;
  const text = reminder(answer.context);
  event.result = { ...event.result, content: [...event.result.content, { type: "text", text }] };
  if (letters.length > 0) holdReceipt(state, key, { root: sessionID, letters, text });
}

/** The shared server has no pane credential. Only the terminal already
 * attached to the actual session's root may run the observation helper. */
async function observeTool(state, ctx, registration, event) {
  const command = event?.input?.command;
  if (event?.tool !== "shell" || typeof command !== "string" || !(command.includes("herdr") || command.includes("HERDR_BIN_PATH"))) return;
  if (!['completed', 'error'].includes(event.status) || typeof event.sessionID !== "string" || typeof event.id !== "string") return;
  if (Buffer.byteLength(command, "utf8") > 64 * 1024) { state.lost += 1; return; }
  const known = await rootOf(state, ctx, event.sessionID);
  const root = known ?? event.sessionID;
  const content = event.result?.content;
  const truncated = Array.isArray(content) && (content.length > 16 || content.reduce((sum, part) => sum + (typeof part?.text === "string" ? Buffer.byteLength(part.text, "utf8") : 0), 0) > 128 * 1024);
  const result = !truncated && Array.isArray(content) && content.every((part) => part?.type === "text" && typeof part.text === "string") ? { content: content.filter((part) => part?.type === "text").map((part) => ({ type: "text", text: part.text })) } : null;
  await ask(state, registration, root, "observation", {
    session_id: event.sessionID,
    observation: { command, call_id: event.id, result, truncated, failed: event.status === "error" },
  }, 1500, known === undefined);
}

/** OpenCode's events: a session's line, and the writes Hide's letters wait for. */
function onEvent(state, registration, event) {
  const type = event?.type;
  const data = event?.data ?? {};
  if (type === "session.created" && typeof data.sessionID === "string") {
    remember(state, state.parents, data.sessionID, data.parentID ?? null);
    return;
  }
  if (type === "session.inbox.delivered" && typeof data.inboxID === "string") {
    const receipt = state.receipts.get(data.inboxID);
    if (receipt) {
      state.receipts.delete(data.inboxID);
      confirm(state, registration, receipt);
    } else {
      remember(state, state.delivered, data.inboxID);
    }
    return;
  }
  if (type === "session.tool.success") {
    const key = toolKey(data.sessionID, data.id);
    const receipt = key && state.receipts.get(key);
    if (!receipt) return;
    state.receipts.delete(key);
    // A later plugin may have rewritten the result: letters it stripped never reached the model, stay pending in
    // Hide and ride the next ask.
    const stored = Array.isArray(data.content) ? data.content : [];
    if (stored.some((item) => item?.type === "text" && typeof item.text === "string" && item.text.includes(receipt.text))) {
      confirm(state, registration, receipt);
    }
  }
}

async function setup(ctx) {
  if (!helperPresent()) return;
  const state = {
    running: new Set(),
    lost: 0,
    closed: false,
    // The terminal attached to each root session, by the id it attached with, and when it last attached.
    attached: new Map(),
    questions: new Map(),
    parents: new Map(),
    receipts: new Map(),
    delivered: new Set(),
    // The confirmation each root's letters wait for, and the letters Hide took.
    confirming: new Map(),
    confirmed: new Set(),
  };
  const registration = await ctx.rpc.register(RPC, {
    // A terminal that opens a session takes it over; one that only renews never takes it from another terminal
    // still attached, so of two terminals showing a session the one that opened it last answers.
    attach: async (input) => {
      if (typeof input?.root !== "string" || typeof input.client !== "string") return {};
      const current = terminalOf(state, input.root);
      if (input.renew !== true || current === null || current === input.client) attachTerminal(state, input.root, input.client);
      return {};
    },
    detach: async (input) => {
      if (typeof input?.root === "string" && state.attached.get(input.root)?.client === input.client) state.attached.delete(input.root);
      return {};
    },
    answer: async (input) => {
      const waiting = typeof input?.id === "string" && state.questions.get(input.id);
      if (waiting) {
        state.questions.delete(input.id);
        waiting(input);
      }
      return {};
    },
  });
  // A throw from any hook but the tool's would end the operator's prompt or fail the call: every failure there
  // leaves OpenCode's work unchanged.
  await ctx.session.hook("prompt", async (event) => {
    try {
      await onPrompt(state, ctx, registration, event);
    } catch {}
  });
  await ctx.tool.hook("execute.before", async (event) => {
    let reason = null;
    try {
      reason = await onTool(state, ctx, registration, event);
    } catch {}
    // The refusal is the one error this plugin raises: OpenCode hands its text to the model.
    if (reason) throw new Error(reason);
  });
  await ctx.tool.hook("execute.after", async (event) => {
    try {
      await onToolResult(state, ctx, registration, event);
    } catch {}
  });
  const events = new AbortController();
  void (async () => {
    try {
      for await (const event of ctx.event.subscribe({ signal: events.signal })) {
        try {
          onEvent(state, registration, event);
        } catch {}
      }
    } catch {}
  })();
  return () => {
    events.abort();
    close(state);
    for (const waiting of state.questions.values()) waiting(null);
    state.questions.clear();
  };
}

export default { id: "hide.opencode", setup };
