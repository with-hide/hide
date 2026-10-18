// hide-opencode-plugin@7 sha256=ccd322861a3007070f42489901b9be756ef1850164fd8d2f7e4809624b56de1e
// Hide's OpenCode 1.x plugin, written by Hide's install kit (hide-agent-hooks).
// An edit is kept and shown as edited in Settings; Reinstall puts Hide's back.
// Outside a Herdr pane Hide manages, or when its helper is gone, it does nothing.
// OpenCode 2 loads this file too and finds nothing to run: its plugin is the `hide-opencode` folder beside this one.

import { spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import { readFileSync, statSync } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";

const HELPER = "/kit path/it's/hide-agent-hooks";
const ROUTE = [];

// The prompt budget is Claude Code's prompt hook's; the tool budget is the
// spawn guard's. OpenCode puts no limit on a hook, so these are the limits.
const PROMPT_BUDGET_MS = 1850;
const TOOL_BUDGET_MS = 2500;
const START_BUDGET_MS = 8000;
const CONFIRM_BUDGET_MS = 2000;
const COUNT_BUDGET_MS = 2000;
// A tool result's letters: the helper's budget, and how often a working session asks. A burst of tool calls asks
// once per interval, so the cost per tool stays bounded, and a letter that is due waits at most this much longer.
const LETTERS_BUDGET_MS = 1500;
const LOOKUP_BUDGET_MS = 300;
const OUTPUT_LIMIT = 64 * 1024;
// Far inside the helper's 256 KiB input, so a pasted log never costs a prompt its letters.
const PROMPT_TEXT_LIMIT = 32 * 1024;
const RUNNING_LIMIT = 8;
const SESSION_LIMIT = 512;
// Any spelling of the reminder tag OpenCode's model reads as one, opening or closing.
const REMINDER_TAG = /<(\s*\/?\s*system-reminder)/gi;
const PENDING_LIMIT = 16;
const PENDING_AGE_MS = 10 * 60 * 1000;

// The project guardrails' policy, which Hide's core writes for this machine (`guardrails::policy_path`). A tool
// call reaches the helper for them only while some project has a protection on, read again when the file changes,
// so a machine with none starts no helper for a file tool (PRD agent-guardrails B27).
const POLICY = path.join(homedir(), ".hide", "agent-hooks", "guardrails.json");
const FILE_TOOLS = new Set(["read", "edit", "write", "apply_patch", "multiedit"]);
// The arguments that name a file; the rest of a write (its content) is not sent.
const PATH_ARGS = ["filePath", "path", "file_path"];
const policy = { mtimeMs: -1, size: -1, on: null };

/** Whether a tool call asks the helper for the guardrails. A policy that is missing or unreadable (`on` is null)
 * does in a Herdr pane, where the helper records why; a session outside one starts no helper for it. */
function guardrailsOn(state) {
  const on = policyOn();
  return on ?? !state.guardOnly;
}

function policyOn() {
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

function helperPresent() {
  try {
    return statSync(HELPER).isFile();
  } catch {
    return false;
  }
}

function managed(env) {
  return env.HERDR_ENV === "1" && Boolean(env.HERDR_PANE_ID) && Boolean(env.HERDR_SOCKET_PATH) && helperPresent();
}

/** The helper's answer, or null when it failed, timed out or said nothing usable. */
function helper(state, operation, input, budgetMs) {
  // Once OpenCode disposed of the plugin, a report still waiting starts nothing.
  if (state.closed) return Promise.resolve(null);
  if (state.running.size >= RUNNING_LIMIT) {
    state.lost += 1;
    return Promise.resolve(null);
  }
  // A call skipped at the cap or given up on reaches Hide's diagnostic through the next one.
  if (state.lost > 0) {
    input = { ...input, lost: state.lost };
    state.lost = 0;
  }
  return new Promise((resolve) => {
    let child;
    try {
      child = spawn(HELPER, ["opencode", operation, ...ROUTE], { stdio: ["pipe", "pipe", "ignore"] });
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

/** A part id in OpenCode's own ascending form (`prt_` + 12 hex + 14 base62), so it sorts after the prompt's parts. */
function partId(state) {
  const now = Date.now();
  state.idCounter = now === state.idTime ? state.idCounter + 1 : 1;
  state.idTime = now;
  const value = BigInt(now) * 0x1000n + BigInt(state.idCounter);
  let hex = "";
  for (let index = 0; index < 6; index += 1) {
    hex += Number((value >> BigInt(40 - 8 * index)) & 0xffn).toString(16).padStart(2, "0");
  }
  const alphabet = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
  let random = "";
  for (const byte of randomBytes(14)) random += alphabet[byte % 62];
  return `prt_${hex}${random}`;
}

/** The start of a prompt, enough for Memory's retrieval, cut where JSON can still carry it. */
function clip(text) {
  // A lone surrogate anywhere would make the JSON the helper reads invalid.
  if (typeof text.toWellFormed === "function") text = text.toWellFormed();
  if (text.length <= PROMPT_TEXT_LIMIT) return text;
  const end = PROMPT_TEXT_LIMIT - (/[\uD800-\uDBFF]/.test(text[PROMPT_TEXT_LIMIT - 1]) ? 1 : 0);
  return text.slice(0, end);
}

function withBudget(promise, budgetMs) {
  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve(null), budgetMs);
    promise.then(
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
 * Records `key` as the newest entry of `map`, a Map or a Set of at most SESSION_LIMIT sessions. At the limit the
 * oldest entry the first matching rule of `evictable` accepts goes, so a long-lived OpenCode keeps tracking its
 * newest sessions; when none may go the new one is not recorded and the loss reaches the diagnostic through the
 * next helper call.
 */
function remember(state, map, key, value, evictable = [() => true]) {
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

/**
 * The session lines that may go at the limit: never the pane's root or a line a running child counts through,
 * a finished child's before a root's, and a root's only when no child's is left.
 */
function parentEviction(state) {
  const kept = new Set([state.root]);
  for (const [child, status] of state.children) {
    if (!status.busy) continue;
    for (let current = child, depth = 0; current && depth < SESSION_LIMIT; depth += 1) {
      kept.add(current);
      current = state.parents.get(current);
    }
  }
  return [(parent, key) => parent !== null && !kept.has(key), (parent, key) => !kept.has(key)];
}

/** Whether `sessionID` is a root session: known from `session.created`, else asked once. A prompted root becomes the newest entry. */
async function rootSession(state, client, sessionID) {
  if (state.parents.has(sessionID)) {
    const parent = state.parents.get(sessionID);
    remember(state, state.parents, sessionID, parent, parentEviction(state));
    return parent === null;
  }
  const answer = await withBudget(client.session.get({ path: { id: sessionID } }), LOOKUP_BUDGET_MS);
  const info = answer?.data;
  if (!info || info.id !== sessionID) return false;
  remember(state, state.parents, sessionID, info.parentID ?? null, parentEviction(state));
  return !info.parentID;
}

/** The root session above `sessionID`, or null when its line is not known. */
function rootOf(state, sessionID) {
  let current = sessionID;
  for (let depth = 0; depth < SESSION_LIMIT; depth += 1) {
    if (!state.parents.has(current)) return null;
    const parent = state.parents.get(current);
    if (parent === null) return current;
    current = parent;
  }
  return null;
}

/**
 * Sends the pane's subagent counts when they changed, or always when `force`. One report runs at a time and only
 * the newest waiting one follows it, so a burst of subagents costs two helper runs, not one per change. The
 * promise settles once the counts known now have been sent.
 */
function publishCounts(state, force) {
  let working = 0;
  let done = 0;
  for (const [child, status] of state.children) {
    if (rootOf(state, child) !== state.root) continue;
    if (status.busy) working += 1;
    else if (status.ran) done += 1;
  }
  const key = `${working}/${done}`;
  if (!force && key === state.published) return state.counting ?? Promise.resolve();
  // Only a report Herdr took counts as published: a failed one is sent again with the next change, prompt or rest.
  state.published = key;
  state.nextCount = { working, done, key };
  state.counting ??= (async () => {
    while (state.nextCount) {
      const next = state.nextCount;
      state.nextCount = null;
      const answer = await helper(state, "subagents", { working: next.working, done: next.done }, COUNT_BUDGET_MS);
      if (answer?.reported !== true && state.published === next.key) state.published = null;
    }
    state.counting = null;
  })();
  return state.counting;
}

/** At root idle, finished children leave the working count; a background child still busy stays. */
async function sweep(state, client) {
  const busy = [...state.children].filter(([child, status]) => status.busy && rootOf(state, child) === state.root);
  if (busy.length === 0) return;
  const answer = await withBudget(client.session.status(), LOOKUP_BUDGET_MS);
  const running = answer?.data;
  if (!running || typeof running !== "object") return;
  for (const [child, status] of busy) {
    const now = running[child]?.type;
    if (now !== "busy" && now !== "retry") status.busy = false;
  }
}

/** Applies one event; the promise settles once what it changed is reported (OpenCode does not wait for it). */
async function onEvent(state, client, event) {
  const type = event?.type;
  const properties = event?.properties ?? {};
  if (type === "session.created") {
    const info = properties.info;
    if (!info?.id) return;
    remember(state, state.parents, info.id, info.parentID ?? null, parentEviction(state));
    // A child still running is never forgotten: it would leave the working count while it works.
    if (info.parentID) remember(state, state.children, info.id, { busy: false, ran: false }, [(child) => !child.busy]);
    return;
  }
  if (type === "session.status") {
    const sessionID = properties.sessionID;
    const kind = properties.status?.type;
    const child = state.children.get(sessionID);
    if (child) {
      const busy = kind === "busy" || kind === "retry";
      child.busy = busy;
      child.ran ||= busy;
      await publishCounts(state, false);
    } else if (sessionID === state.root && kind === "idle") {
      // Each turn's end reports again, as Claude Code's Stop does, so a Herdr that lost the pane's counts gets them back.
      await sweep(state, client);
      await publishCounts(state, true);
    }
    return;
  }
  if (type === "message.part.updated") {
    const part = properties.part;
    // Hide's own text part is written once; a tool part is updated through its run and carries the letters its
    // result got once it is completed.
    const key = part?.type === "tool" ? (part.state?.status === "completed" ? toolKey(part.sessionID, part.callID) : undefined) : part?.id;
    const pending = key && state.pending.get(key);
    if (!pending) return;
    // A tool part is Hide's letters' only when the stored output still carries them: a later plugin may have
    // rewritten the result, and a child session's call may share the id.
    state.pending.delete(key);
    // The letters a plugin after this one stripped from the stored result never reached the model: they stay
    // pending in Hide and ride the next ask.
    if (pending.text && !(typeof part.state?.output === "string" && part.state.output.includes(pending.text))) return;
    // OpenCode wrote the prompt or the tool result carrying the letters: only now are they received.
    confirmLetters(state, pending.session, pending.letters);
  }
}

/** Confirms the letters OpenCode stored; a letter Hide did not take stays pending there and rides the next ask. */
function confirmLetters(state, session, letters) {
  for (const letter of letters) state.confirming.add(letter);
  void helper(state, "confirm", { session_id: session, letters }, CONFIRM_BUDGET_MS).then((answer) => {
    const confirmed = Array.isArray(answer?.confirmed) ? answer.confirmed : [];
    for (const letter of letters) {
      state.confirming.delete(letter);
      if (confirmed.includes(letter)) remember(state, state.confirmed, letter);
    }
  });
}

/** The pending key of a tool part: its session and call, since a child session may reuse a call id. */
function toolKey(sessionID, callID) {
  return typeof sessionID === "string" && typeof callID === "string" ? `${sessionID}\u0000${callID}` : undefined;
}

/** Forgets the writes that never came; the letters they carried stay pending in Hide and ride the next ask. */
function forgetStale(state, now) {
  for (const [name, old] of state.pending) {
    if (now - old.at > PENDING_AGE_MS) state.pending.delete(name);
  }
}

/** Whether letters this plugin handed over still wait for their receipt, OpenCode's write or Hide's confirmation. */
function handoverOpen(state) {
  forgetStale(state, Date.now());
  if (state.confirming.size > 0) return true;
  for (const entry of state.pending.values()) {
    if (entry.letters.length > 0) return true;
  }
  return false;
}

/** Makes room for `key`; a letter left out stays pending in Hide. */
function holdPending(state, key, entry) {
  const now = Date.now();
  forgetStale(state, now);
  if (state.pending.size >= PENDING_LIMIT) state.pending.delete(state.pending.keys().next().value);
  state.pending.set(key, { ...entry, at: now });
}

/**
 * The letters due while the pane's agent works, written after the tool's result so the agent reads them before its
 * next step (PRD letters-to-busy-agents D-02); confirmed once OpenCode stored the completed tool part of that session
 * with the letters still in its output. Only the pane's root session asks, at every tool result (a `--now` letter
 * goes at the next one, D-05), except while letters it handed over still wait for their receipt: those are not
 * written twice, and the receipt comes with the stored part, before the next result in practice.
 */
async function onToolResult(state, sessionID, input, output) {
  if (!sessionID || sessionID !== state.root || typeof input?.callID !== "string") return;
  if (handoverOpen(state)) return;
  const answer = await helper(state, "letters", { session_id: sessionID }, LETTERS_BUDGET_MS);
  if (typeof answer?.context !== "string" || !answer.context) return;
  const letters = Array.isArray(answer.letters) ? answer.letters.filter((letter) => typeof letter === "string") : [];
  // A letter Hide already took is never written twice.
  if (letters.some((letter) => state.confirmed.has(letter))) return;
  const text = `<system-reminder>\n${answer.context.replace(REMINDER_TAG, "<\u200b$1")}\n</system-reminder>`;
  output.output = typeof output.output === "string" && output.output ? `${output.output}\n\n${text}` : text;
  if (letters.length > 0) holdPending(state, toolKey(sessionID, input.callID), { session: sessionID, letters, text });
}

async function onPrompt(state, client, directory, input, output) {
  const sessionID = input?.sessionID;
  const message = output?.message;
  const parts = output?.parts;
  if (!sessionID || !message?.id || !Array.isArray(parts)) return;
  // A prompt that is only text OpenCode or a plugin wrote is no one's turn.
  const typed = parts.filter((part) => !(part?.type === "text" && part.synthetic));
  if (typed.length === 0) return;
  const deadline = Date.now() + PROMPT_BUDGET_MS;
  if (!(await rootSession(state, client, sessionID))) return;
  // A session's first prompt this plugin sees carries the start guidance and the session-start Memory, as
  // Claude Code's SessionStart does for a new or resumed session; event order cannot hide it.
  const first = !state.guided.has(sessionID);
  state.root = sessionID;
  const prompt = clip(
    typed
      .filter((part) => part.type === "text" && typeof part.text === "string")
      .map((part) => part.text)
      .join("\n"),
  );
  const answer = await helper(state, "prompt", { session_id: sessionID, prompt, cwd: directory, first }, Math.max(0, deadline - Date.now()));
  // After the prompt's own helper, so the report never shares its budget: each turn's start reports again.
  void publishCounts(state, true);
  const sections = [];
  // Guidance and the session-start Memory travel together: with no answer neither is given, and the next prompt asks again.
  if (first && answer !== null) {
    const guidance = await withBudget(state.guidance, Math.max(0, deadline - Date.now()));
    if (typeof guidance?.context === "string" && guidance.context) sections.push(guidance.context);
    // Guidance still on its way is given to the next prompt instead.
    if (state.started) remember(state, state.guided, sessionID);
  }
  if (typeof answer?.context === "string" && answer.context) sections.push(answer.context);
  if (sections.length === 0) return;
  const id = partId(state);
  // Synthetic: OpenCode's screen leaves it out of the operator's message, and the reminder tags tell the model it is not the operator's text.
  // Hide writes those tags itself, so a letter or Memory line cannot close them and speak as the operator.
  const text = `<system-reminder>\n${sections.join("\n\n").replace(REMINDER_TAG, "<\u200b$1")}\n</system-reminder>`;
  parts.push({ id, sessionID, messageID: message.id, type: "text", text, synthetic: true });
  const letters = Array.isArray(answer?.letters) ? answer.letters.filter((letter) => typeof letter === "string") : [];
  if (letters.length > 0) holdPending(state, id, { session: sessionID, letters });
}

async function onTool(state, sessionID, directory, input, output) {
  const tool = input?.tool;
  let request;
  if (tool === "bash") {
    const command = output?.args?.command;
    // Every shell call comes here: one asks the helper while a guardrail is on, or, in a Herdr pane, when it can
    // start an agent through Herdr.
    if (typeof command !== "string") return null;
    const launch = !state.guardOnly && (command.includes("herdr") || command.includes("HERDR_BIN_PATH"));
    if (!launch && !guardrailsOn(state)) return null;
    // OpenCode runs a relative `workdir` in the session's folder, not where this process started.
    const workdir = output.args.workdir;
    const cwd = typeof workdir === "string" && workdir ? (typeof directory === "string" ? path.resolve(directory, workdir) : workdir) : directory;
    request = { session_id: sessionID, tool, command, cwd };
  } else if (FILE_TOOLS.has(tool)) {
    if (!guardrailsOn(state)) return null;
    request = { session_id: sessionID, tool, args: fileArgs(output?.args), cwd: directory };
  } else if (tool === "question" && !state.guardOnly) {
    // The core knows the pane by the root session Herdr's integration reports, also for a subagent's call.
    request = { session_id: rootOf(state, sessionID) ?? sessionID, tool };
  } else {
    return null;
  }
  // A guard-only session asks for the guardrails alone, whatever pane its shell inherited.
  const answer = await helper(state, "tool", state.guardOnly ? { ...request, guard_only: true } : request, TOOL_BUDGET_MS);
  return typeof answer?.deny === "string" && answer.deny ? answer.deny : null;
}

const HidePlugin = async ({ client, directory }) => {
  // Outside a Herdr pane Hide has no letters, counts or guidance for the session, but a guarded project's
  // protections hold there too (PRD agent-guardrails B22).
  if (!managed(process.env)) return helperPresent() ? guardOnly(directory) : {};
  const state = {
    running: new Set(),
    lost: 0,
    parents: new Map(),
    children: new Map(),
    guided: new Set(),
    started: false,
    pending: new Map(),
    confirming: new Set(),
    confirmed: new Set(),
    root: null,
    published: null,
    closed: false,
    counting: null,
    nextCount: null,
    idTime: 0,
    idCounter: 0,
    guidance: null,
  };
  state.guidance = helper(state, "start", { cwd: directory }, START_BUDGET_MS).then((answer) => {
    state.started = true;
    return answer;
  });
  void publishCounts(state, true);
  return {
    // OpenCode 1.18.30 calls this as `void hook["event"]?.(...)` (packages/opencode/src/plugin/index.ts) and never
    // waits for it, so the reports it awaits hold no other event back.
    event: async ({ event }) => {
      try {
        await onEvent(state, client, event);
      } catch {}
    },
    "chat.message": async (input, output) => {
      // A throw here would end the operator's prompt: every failure passes the prompt unchanged.
      try {
        await onPrompt(state, client, directory, input, output);
      } catch {}
    },
    "tool.execute.before": async (input, output) => {
      let reason = null;
      try {
        reason = await onTool(state, input?.sessionID, directory, input, output);
      } catch {}
      // The refusal is the one error this plugin raises: OpenCode hands its text to the model.
      if (reason) throw new Error(reason);
    },
    "tool.execute.after": async (input, output) => {
      // A throw here would fail the tool call: every failure leaves its result unchanged.
      try {
        await onToolResult(state, input?.sessionID, input, output);
      } catch {}
    },
    dispose: async () => {
      state.closed = true;
      for (const child of state.running) child.kill("SIGKILL");
      state.running.clear();
    },
  };
};

/** The plugin outside a Herdr pane: the guardrails' tool check and nothing else. */
function guardOnly(directory) {
  const state = { running: new Set(), lost: 0, closed: false, guardOnly: true };
  return {
    "tool.execute.before": async (input, output) => {
      let reason = null;
      try {
        reason = await onTool(state, input?.sessionID, directory, input, output);
      } catch {}
      if (reason) throw new Error(reason);
    },
    dispose: async () => {
      state.closed = true;
      for (const child of state.running) child.kill("SIGKILL");
      state.running.clear();
    },
  };
}

// OpenCode 1.x runs `server`; OpenCode 2 reads the same default as a plugin whose setup does nothing.
export default { id: "hide.opencode.v1", server: HidePlugin, setup() {} };
