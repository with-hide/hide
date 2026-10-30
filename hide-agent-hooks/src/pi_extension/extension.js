// Hide's extension for Pi and omp, written by Hide's install kit (hide-agent-hooks).
// An edit is kept and shown as edited in Settings; Reinstall puts Hide's back.
// Outside a Herdr pane, in an agent started from another Pi's or omp's shell, or when its helper is gone, it does
// nothing; inside one it acts for the pane's own TUI session, and its spawn guard for every session in it.
// @ts-nocheck

import { spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import { readFileSync, readlinkSync, statSync } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";

const HELPER = __HIDE_HELPER__;
const ROUTE = __HIDE_ROUTE__;
const AGENT = __HIDE_AGENT__;
const VERSION = __HIDE_VERSION__;

// The prompt budget is Claude Code's prompt hook's; the tool budget is the spawn guard's. Both hosts wait for
// a handler far longer (omp gives up at 30 s and then blocks the tool), so these are the limits.
const PROMPT_BUDGET_MS = 1850;
const TOOL_BUDGET_MS = 2500;
const START_BUDGET_MS = 8000;
const CONFIRM_BUDGET_MS = 2000;
const COUNT_BUDGET_MS = 2000;
// A tool result's letters: the helper's budget, and how often the pane's agent asks while it works. A burst of tool
// calls asks once per interval, so the cost per tool stays bounded, and a letter that is due waits at most this
// much longer.
const LETTERS_BUDGET_MS = 1500;
const OUTPUT_LIMIT = 64 * 1024;
// Far inside the helper's 256 KiB input, so a pasted log never costs a prompt its letters.
const PROMPT_TEXT_LIMIT = 32 * 1024;
const RUNNING_LIMIT = 8;
const SESSION_LIMIT = 512;
const CHILD_LIMIT = 64;
const PENDING_LIMIT = 16;
const PENDING_AGE_MS = 10 * 60 * 1000;
const CUSTOM_TYPE = "hide";

// The project guardrails' policy, which Hide's core writes for this machine (`guardrails::policy_path`). A tool
// call reaches the helper for them only while some project has a protection on, read again when the file changes,
// so a machine with none starts no helper for a file tool (PRD agent-guardrails B27).
const POLICY = path.join(homedir(), ".hide", "agent-hooks", "guardrails.json");
// A call naming the Hide global instruction always asks the helper, which refuses its write whatever the guardrails
// say (PRD memory-self-improve D-07); the name is matched as the helper's guard matches it.
const GLOBAL_INSTRUCTION = "global-instruction.md";
const FILE_TOOLS = new Set(["read", "edit", "write"]);
// The arguments that name a file; the rest of a write (its content) is not sent.
const PATH_ARGS = ["filePath", "path", "file_path"];
const policy = { mtimeMs: -1, size: -1, on: null };

// The state folders the helper's guard knows: the one Hide installed this file for, `HIDE_STATE_DIR`,
// `$XDG_STATE_HOME/hide` and `~/.hide/state`.
const STATE_DIRS = [
  ROUTE[0] === "--state-dir-b64" ? Buffer.from(ROUTE[1], "base64url").toString() : "",
  process.env.HIDE_STATE_DIR ?? "",
  process.env.XDG_STATE_HOME ? path.join(process.env.XDG_STATE_HOME, "hide") : "",
  path.join(homedir(), ".hide", "state"),
].filter(Boolean);

/** Whether a shell command can hand a result to the system opener: a word of an opener and a target Hide's View
 * shows, the helper's own byte test (`opener_guard::may_hold_opening`), whose reading of the call decides. */
const OPENER_WORD = /open|start|saps|invoke-item|explorer|browser|wslview|(?:^|[^a-z0-9])ii(?![a-z0-9])/i;
const VIEWABLE = /https?:|file:|\.(?:html?|png|jpe?g|gif|webp|bmp|avif|svg|ico|pdf|md|markdown|mdx|txt|log|csv|json|ya?ml|mp4|webm|mov)(?![a-z0-9])/i;
function mayOpen(command) {
  return OPENER_WORD.test(command) && VIEWABLE.test(command);
}

/** Whether `text` can name the Hide global instruction, or the file a link at it points to, which is the one every
 * session is given; names are matched as the helper's guard matches them. */
function namesGlobal(text) {
  if (typeof text !== "string") return false;
  const lower = text.toLowerCase();
  if (lower.includes(GLOBAL_INSTRUCTION)) return true;
  return STATE_DIRS.some((dir) => {
    let target;
    try {
      target = readlinkSync(path.join(dir, GLOBAL_INSTRUCTION));
    } catch {
      return false;
    }
    const name = path.basename(target).toLowerCase();
    return name !== "" && lower.includes(name);
  });
}

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
// Any spelling of the reminder tag the model reads as one, opening or closing.
const REMINDER_TAG = /<(\s*\/?\s*system-reminder)/gi;

// omp's subagents run in the same process and get their own binding of this factory, sharing the module: the
// counts live here, and the main session's binding reports them.
const shared = { children: new Map(), publish: null, rootSession: null };

function helperPresent() {
  try {
    return statSync(HELPER).isFile();
  } catch {
    return false;
  }
}

function managed(env) {
  if (env.HERDR_ENV !== "1" || !env.HERDR_PANE_ID || !env.HERDR_SOCKET_PATH) return false;
  // omp marks every shell it starts with OMPCODE=1 and Pi with PI_SESSION_ID: an agent started there is not
  // the pane's agent, and its prompts must not take the pane's letters.
  if (env.OMPCODE === "1" || env.PI_SESSION_ID) return false;
  return helperPresent();
}

/** The helper's answer, or null when it failed, timed out or said nothing usable. */
function helper(state, operation, input, budgetMs) {
  if (state.closed && operation !== "confirm" && operation !== "subagents") return Promise.resolve(null);
  if (state.running.size >= RUNNING_LIMIT) {
    state.lost += 1;
    return Promise.resolve(null);
  }
  // A call skipped at the cap or given up on reaches Hide's diagnostic through the next one.
  input = { ...input, version: VERSION };
  if (state.lost > 0) {
    input.lost = state.lost;
    state.lost = 0;
  }
  return new Promise((resolve) => {
    let child;
    try {
      child = spawn(HELPER, [AGENT, operation, ...ROUTE], { stdio: ["pipe", "pipe", "ignore"] });
    } catch {
      state.lost += 1;
      resolve(null);
      return;
    }
    child.hideOperation = operation;
    state.running.add(child);
    const chunks = [];
    let size = 0;
    let settled = false;
    let timer = null;
    const finish = (value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      state.running.delete(child);
      resolve(value);
    };
    // A throw in a timer would end omp's session: every callback here catches its own.
    timer = setTimeout(() => {
      try {
        child.kill("SIGKILL");
      } catch {}
      state.lost += 1;
      finish(null);
    }, budgetMs);
    try {
      child.on("error", () => {
        if (!settled) state.lost += 1;
        finish(null);
      });
      child.stdout.on("error", () => finish(null));
      child.stdout.on("data", (chunk) => {
        size += chunk.length;
        if (size > OUTPUT_LIMIT) {
          try {
            child.kill("SIGKILL");
          } catch {}
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
    } catch {
      try {
        child.kill("SIGKILL");
      } catch {}
      finish(null);
    }
  });
}

function withBudget(promise, budgetMs) {
  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve(null), Math.max(0, budgetMs));
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

/** The start of a prompt, enough for Memory's retrieval, cut where JSON can still carry it. */
function clip(text) {
  if (typeof text !== "string") return "";
  // A lone surrogate anywhere would make the JSON the helper reads invalid.
  if (typeof text.toWellFormed === "function") text = text.toWellFormed();
  if (text.length <= PROMPT_TEXT_LIMIT) return text;
  const end = PROMPT_TEXT_LIMIT - (/[\uD800-\uDBFF]/.test(text[PROMPT_TEXT_LIMIT - 1]) ? 1 : 0);
  return text.slice(0, end);
}

/** Whether `ctx` is one of omp's subagent sessions. */
function subagent(ctx) {
  return ctx?.agent?.kind === "sub";
}

/**
 * Whether `ctx` is the pane's own agent: the host's TUI, the one session drawn in the pane. Both hosts' RPC modes
 * report a UI too but draw nothing there, and another program in the pane may start one.
 */
function root(ctx) {
  return !subagent(ctx) && ctx?.mode === "tui";
}

/**
 * The session file Herdr's integration reports for the pane, or null for an unsaved session. Absolute in either
 * system's spelling, as Herdr's own Pi integration judges it: Windows reports `C:\...`, which the host's `path` would
 * only call absolute on Windows.
 */
function sessionFile(ctx) {
  try {
    const file = ctx?.sessionManager?.getSessionFile?.();
    return typeof file === "string" && (path.posix.isAbsolute(file) || path.win32.isAbsolute(file)) ? file : null;
  } catch {
    return null;
  }
}

/** The host's own id for the session, which Memory knows it by, or null. */
function nativeSession(ctx) {
  try {
    const id = ctx?.sessionManager?.getSessionId?.();
    return typeof id === "string" && id.length > 0 && id.length <= 4096 ? id : null;
  } catch {
    return null;
  }
}

/** Records `key` as the newest member of a Set of at most SESSION_LIMIT, dropping the oldest. */
function remember(set, key) {
  set.delete(key);
  if (set.size >= SESSION_LIMIT) set.delete(set.values().next().value);
  set.add(key);
}

/**
 * Sends the pane's subagent counts when they changed, or always when `force`. One report runs at a time and only
 * the newest waiting one follows it, so a burst of subagents costs two helper runs, not one per change.
 */
function publishCounts(state, force) {
  let working = 0;
  let done = 0;
  for (const child of shared.children.values()) {
    if (child.busy) working += 1;
    else if (child.ran) done += 1;
  }
  const key = `${working}/${done}`;
  if (!force && key === state.published) return state.counting ?? Promise.resolve();
  // Only a report Herdr took counts as published: a failed one is sent again with the next change, prompt or rest.
  state.published = key;
  state.nextCount = { working, done, key };
  state.counting ??= (async () => {
    try {
      while (state.nextCount) {
        const next = state.nextCount;
        state.nextCount = null;
        const answer = await helper(state, "subagents", { working: next.working, done: next.done }, COUNT_BUDGET_MS);
        if (answer?.reported !== true && state.published === next.key) state.published = null;
      }
    } finally {
      state.counting = null;
    }
  })();
  return state.counting;
}

/** One subagent's line: a spawn the parent dispatched, then its own session's turns. */
function child(id) {
  let entry = shared.children.get(id);
  if (entry) return entry;
  if (shared.children.size >= CHILD_LIMIT) {
    // A child still running is never forgotten: it would leave the working count while it works.
    const done = [...shared.children].find(([, line]) => !line.busy);
    if (!done) return null;
    shared.children.delete(done[0]);
  }
  entry = { busy: false, ran: false };
  shared.children.set(id, entry);
  return entry;
}

function setChild(id, busy) {
  if (typeof id !== "string" || !id) return;
  const entry = child(id);
  if (!entry) return;
  entry.busy = busy;
  entry.ran ||= busy;
  void shared.publish?.(false);
}

async function onPrompt(state, event, ctx) {
  if (!root(ctx)) return undefined;
  const session = sessionFile(ctx);
  if (!session) return undefined;
  shared.rootSession = session;
  const deadline = Date.now() + PROMPT_BUDGET_MS;
  // Until the host writes a message that carried it, a session's prompts carry the start guidance, as Claude Code's
  // SessionStart does for a new or resumed session.
  const first = !state.guided.has(session);
  if (first) state.guidance ??= startGuidance(state, ctx);
  // Memory's session-start capsule is its own: it rides one written message per session, as Claude Code's SessionStart
  // capsule does, whether or not the guidance has arrived yet, and the prompt capsule follows it.
  const memoryFirst = !state.memoryStarted.has(session);
  const answer = await helper(
    state,
    "prompt",
    {
      session_id: session,
      native_session: nativeSession(ctx),
      prompt: clip(event?.prompt),
      cwd: ctx?.cwd,
      first,
      memory_first: memoryFirst,
    },
    Math.max(0, deadline - Date.now()),
  );
  const sections = [];
  let guided = false;
  if (first && answer !== null) {
    // Guidance still on its way is given to the next prompt instead.
    const guidance = await withBudget(state.guidance, deadline - Date.now());
    if (typeof guidance?.context === "string" && guidance.context) {
      sections.push(guidance.context);
      guided = true;
    }
  }
  if (typeof answer?.context === "string" && answer.context) sections.push(answer.context);
  if (sections.length === 0) return undefined;
  // omp can prepare one submission more than once and keeps only the attempt it delivers: each attempt carries its
  // own id, and only the one whose message the host writes is confirmed.
  const id = randomBytes(8).toString("hex");
  const letters = Array.isArray(answer?.letters) ? answer.letters.filter((letter) => typeof letter === "string") : [];
  const memoryStarted = memoryFirst && answer?.memory_start === true;
  holdPending(state, id, { session, letters, guided, memoryStarted });
  // Hidden: the host keeps it out of the conversation on screen and out of the title, and the reminder tags tell the
  // model it is not the operator's text. Hide writes those tags itself, so a letter cannot close them.
  const text = `<system-reminder>\n${sections.join("\n\n").replace(REMINDER_TAG, "<\u200b$1")}\n</system-reminder>`;
  return { message: { customType: CUSTOM_TYPE, content: text, display: false, details: { hide: id } } };
}

/** Forgets the writes and replies that never came; the letters they carried stay pending in Hide and come again. */
function forgetStale(state, now) {
  for (const [name, old] of state.pending) {
    if (now - old.at > PENDING_AGE_MS) state.pending.delete(name);
  }
  state.written = state.written.filter((entry) => now - entry.at <= PENDING_AGE_MS);
}

/** Makes room for `key`; a letter left out stays pending in Hide and reaches the next ask, and guidance left out comes again. */
function holdPending(state, key, entry) {
  const now = Date.now();
  forgetStale(state, now);
  if (state.pending.size >= PENDING_LIMIT) state.pending.delete(state.pending.keys().next().value);
  state.pending.set(key, { ...entry, at: now });
}

/** Whether letters this session handed over still wait for their receipt, the host's write or Hide's confirmation. */
function handoverOpen(state) {
  forgetStale(state, Date.now());
  if (state.confirming.size > 0) return true;
  for (const entry of [...state.pending.values(), ...state.written]) {
    if (entry.letters.length > 0) return true;
  }
  return false;
}

/**
 * The letters due while the pane's agent works, added to the tool's result so the agent reads them before its next
 * step (PRD letters-to-busy-agents D-02); confirmed as a prompt's letters are, once the host has written the result
 * with Hide's text still in it and then the reply that follows (`message_end` can still replace the result itself).
 * Only the pane's own agent asks, at every tool result (a `--now` letter goes at the next one, D-05), except while
 * letters it handed over still wait for their receipt: those are not added twice, and the receipt comes with the
 * reply, before the next result.
 */
async function onToolResult(state, event, ctx) {
  await observeTool(state, event, ctx);
  if (!root(ctx) || typeof event?.toolCallId !== "string") return undefined;
  const session = sessionFile(ctx);
  if (!session) return undefined;
  if (handoverOpen(state)) return undefined;
  const answer = await helper(state, "letters", { session_id: session, native_session: nativeSession(ctx) }, LETTERS_BUDGET_MS);
  if (typeof answer?.context !== "string" || !answer.context) return undefined;
  const letters = Array.isArray(answer.letters) ? answer.letters.filter((letter) => typeof letter === "string") : [];
  // A letter Hide already took is never added twice.
  if (letters.some((letter) => state.confirmed.has(letter))) return undefined;
  const text = `<system-reminder>\n${answer.context.replace(REMINDER_TAG, "<\u200b$1")}\n</system-reminder>`;
  if (letters.length > 0) holdPending(state, event.toolCallId, { session, letters, text });
  const content = [...(Array.isArray(event.content) ? event.content : []), { type: "text", text }];
  // Replacing the content alone drops a tool's structured result: it is returned with it.
  return event.structuredContent === undefined ? { content } : { content, structuredContent: event.structuredContent };
}

/** `tool_result` includes the executed input in both hosts. Internal omp
 * sessions keep their own native id; a parent's file is never substituted.
 * The daemon may reject a child it cannot attest. */
async function observeTool(state, event, ctx) {
  const command = event?.input?.command;
  if (!["bash", "powershell"].includes(event?.toolName) || typeof command !== "string" || !(command.includes("herdr") || command.includes("HERDR_BIN_PATH"))) return;
  const session = nativeSession(ctx);
  if (!session || typeof event.toolCallId !== "string" || Buffer.byteLength(command, "utf8") > 64 * 1024) { state.lost += 1; return; }
  const content = event.content;
  const truncated = Array.isArray(content) && (content.length > 16 || content.reduce((sum, part) => sum + (typeof part?.text === "string" ? Buffer.byteLength(part.text, "utf8") : 0), 0) > 128 * 1024);
  // Preserve Pi's separate structured result when it actually is the Herdr
  // envelope, without serializing arbitrary tool metadata or prompt bodies.
  const structured = AGENT === "pi" && event.structuredContent?.id === "cli:agent:prompt" && !event.structuredContent?.error && event.structuredContent?.result?.type === "agent_prompted" ? event.structuredContent : null;
  let result = null;
  if (structured) {
    const agent = structured.result.agent;
    const reference = agent?.agent_session;
    const identifiers = [agent?.pane_id, agent?.terminal_id, reference?.source, reference?.agent, reference?.kind, reference?.value];
    if (identifiers.every((value) => value === undefined || value === null || (typeof value === "string" && value.length <= 4096))) {
      result = { id: "cli:agent:prompt", result: { type: "agent_prompted", agent: { pane_id: agent?.pane_id, terminal_id: agent?.terminal_id, agent_session: reference ? { source: reference.source, agent: reference.agent, kind: reference.kind, value: reference.value } : null } } };
    }
  } else if (!truncated && Array.isArray(content) && content.every((part) => part?.type === "text" && typeof part.text === "string")) {
    result = { content: content.filter((part) => part?.type === "text").map((part) => ({ type: "text", text: part.text })) };
  }
  await helper(state, "observation", {
    session_id: session,
    observation: { command, call_id: event.toolCallId, result, truncated,
      failed: event.isError === true, asynchronous: Boolean(event.details?.background || event.details?.asyncJobId) },
  }, 1500);
}

/** A message the host finished: Hide's hidden message is written, and the reply after it means the turn is recorded. */
function onMessageEnd(state, event, ctx) {
  if (!root(ctx)) return;
  const message = event?.message;
  if (message?.role === "custom" && message.customType === CUSTOM_TYPE) {
    const id = message.details?.hide;
    const pending = typeof id === "string" ? state.pending.get(id) : null;
    if (pending) {
      state.pending.delete(id);
      if (pending.guided) remember(state.guided, pending.session);
      if (pending.memoryStarted) remember(state.memoryStarted, pending.session);
      if (pending.letters.length > 0) state.written.push(pending);
    }
    return;
  }
  if (message?.role === "toolResult") {
    // The host finished the tool result carrying the letters; the reply after it proves it was kept and read. A
    // result a later handler replaced without Hide's text never reached the model: its letters stay pending in Hide
    // and ride the next ask.
    const id = message.toolCallId;
    const pending = typeof id === "string" ? state.pending.get(id) : null;
    if (pending) {
      state.pending.delete(id);
      if (kept(message.content, pending.text)) state.written.push(pending);
    }
    return;
  }
  if (message?.role !== "assistant" || state.written.length === 0) return;
  // The host wrote the prompt and Hide's message before the reply: only now are the letters received.
  const written = state.written;
  state.written = [];
  confirmWritten(state, written);
}

/** Whether a finished tool result's `content` still carries Hide's `text`. */
function kept(content, text) {
  return Array.isArray(content) && content.some((part) => part?.type === "text" && typeof part.text === "string" && part.text.includes(text));
}

/** Confirms the letters of the written messages once each; a letter Hide did not take rides the next ask again. */
function confirmWritten(state, written) {
  for (const { session, letters } of written) {
    const fresh = letters.filter((letter) => !state.confirmed.has(letter) && !state.confirming.has(letter));
    if (fresh.length === 0) continue;
    for (const letter of fresh) state.confirming.add(letter);
    // Only a confirmation Hide answered counts: a letter it did not take stays pending there, rides the next prompt
    // again and is confirmed then.
    void helper(state, "confirm", { session_id: session, letters: fresh }, CONFIRM_BUDGET_MS).then((answer) => {
      const confirmed = Array.isArray(answer?.confirmed) ? answer.confirmed : [];
      for (const letter of fresh) {
        state.confirming.delete(letter);
        if (confirmed.includes(letter)) remember(state.confirmed, letter);
      }
    });
  }
}

async function onTool(state, event, ctx) {
  const tool = event?.toolName;
  let request;
  if (tool === "bash") {
    const command = event?.input?.command;
    // Every shell call comes here: one asks the helper while a guardrail is on, or, in the pane's own agent, when
    // it can start an agent through Herdr or hand a result to the system opener.
    if (typeof command !== "string") return null;
    const launch = !state.guardOnly && (command.includes("herdr") || command.includes("HERDR_BIN_PATH") || mayOpen(command));
    if (!launch && !namesGlobal(command) && !guardrailsOn(state)) return null;
    const cwd = typeof event.input.cwd === "string" && event.input.cwd ? event.input.cwd : ctx?.cwd;
    request = { tool, command, cwd };
  } else if (FILE_TOOLS.has(tool)) {
    if (!namesGlobal(JSON.stringify(event?.input ?? {})) && !guardrailsOn(state)) return null;
    request = { tool, args: fileArgs(event?.input), cwd: ctx?.cwd };
  } else {
    return null;
  }
  // A guard-only session asks for the guardrails alone, whatever pane its shell inherited.
  const answer = await helper(state, "tool", state.guardOnly ? { ...request, guard_only: true } : request, TOOL_BUDGET_MS);
  return typeof answer?.deny === "string" && answer.deny ? answer.deny : null;
}

/** The session guidance, read once; a read that failed is asked again by the next prompt that needs it. */
function startGuidance(state, ctx) {
  const reading = helper(state, "start", { cwd: ctx?.cwd }, START_BUDGET_MS).then((answer) => {
    if (answer === null && state.guidance === reading) state.guidance = null;
    return answer;
  });
  return reading;
}

function onSessionStart(state, ctx) {
  if (subagent(ctx)) {
    setChild(ctx.agent.id, true);
    return;
  }
  if (!root(ctx)) return;
  state.root = true;
  shared.rootSession = sessionFile(ctx);
  state.guidance ??= startGuidance(state, ctx);
  if (AGENT === "omp") {
    // A new session in the pane starts its counts at zero, as Claude Code's SessionStart does.
    shared.children.clear();
    shared.publish = (force) => publishCounts(state, force);
    void publishCounts(state, true);
  }
}

/** Runs `handler` so that nothing it throws or rejects with reaches the host. */
function quiet(handler) {
  return async (event, ctx) => {
    try {
      return await handler(event, ctx);
    } catch {
      return undefined;
    }
  };
}

export default function hide(pi) {
  if (typeof pi?.on !== "function") return;
  // Outside the pane's own agent Hide has no letters, counts or guidance for the session, but a guarded project's
  // protections hold there too (PRD agent-guardrails B22).
  if (!managed(process.env)) {
    if (helperPresent()) guardOnly(pi);
    return;
  }
  const state = {
    running: new Set(),
    lost: 0,
    closed: false,
    root: false,
    guidance: null,
    guided: new Set(),
    memoryStarted: new Set(),
    pending: new Map(),
    written: [],
    confirmed: new Set(),
    confirming: new Set(),
    published: null,
    counting: null,
    nextCount: null,
  };
  const on = (name, handler) => {
    try {
      pi.on(name, quiet(handler));
    } catch {}
  };
  on("session_start", (_event, ctx) => onSessionStart(state, ctx));
  // omp reports a session the operator switched to with its own event.
  on("session_switch", (_event, ctx) => onSessionStart(state, ctx));
  // Past the budget the prompt goes on unchanged, and letters it would have carried stay pending in Hide.
  on("before_agent_start", async (event, ctx) => (await withBudget(onPrompt(state, event, ctx), PROMPT_BUDGET_MS)) ?? undefined);
  on("message_end", (event, ctx) => onMessageEnd(state, event, ctx));
  // Past the budget the result goes on unchanged, and the letters it would have carried stay pending in Hide.
  on("tool_result", async (event, ctx) => (await withBudget(onToolResult(state, event, ctx), LETTERS_BUDGET_MS + 200)) ?? undefined);
  on("tool_call", async (event, ctx) => {
    let reason = null;
    try {
      reason = await onTool(state, event, ctx);
    } catch {}
    // A refusal is the one answer this extension blocks with: the host hands its reason to the model.
    return reason ? { block: true, reason } : undefined;
  });
  on("agent_start", (_event, ctx) => {
    if (subagent(ctx)) setChild(ctx.agent.id, true);
  });
  on("agent_end", (_event, ctx) => {
    if (subagent(ctx)) {
      setChild(ctx.agent.id, false);
      return;
    }
    if (!root(ctx)) return;
    // A turn whose reply never came leaves its letters pending in Hide for the next prompt.
    state.written = [];
    // Each turn's end reports again, as Claude Code's Stop does, so a Herdr that lost the pane's counts gets them back.
    if (AGENT === "omp") void publishCounts(state, true);
  });
  on("before_subagent_spawn", (event, ctx) => {
    if (!root(ctx)) return undefined;
    // The spawn's key is the subagent's id: it counts as working from dispatch.
    setChild(event?.spawnKey, true);
    return undefined;
  });
  on("session_shutdown", (_event, ctx) => {
    if (subagent(ctx)) {
      setChild(ctx.agent.id, false);
      return;
    }
    if (!state.root) return;
    state.closed = true;
    if (shared.publish && AGENT === "omp") shared.publish = null;
    // A prompt or tool call still asking has no one to answer; a confirmation or a count finishes on its own budget.
    for (const running of state.running) {
      if (["prompt", "tool", "start", "observation"].includes(running.hideOperation)) {
        try {
          running.kill("SIGKILL");
        } catch {}
      }
    }
  });
}

/** The extension outside the pane's own agent: the guardrails' tool check and nothing else. */
function guardOnly(pi) {
  const state = { running: new Set(), lost: 0, closed: false, guardOnly: true };
  try {
    pi.on(
      "tool_call",
      quiet(async (event, ctx) => {
        let reason = null;
        try {
          reason = await onTool(state, event, ctx);
        } catch {}
        return reason ? { block: true, reason } : undefined;
      }),
    );
    pi.on(
      "session_shutdown",
      quiet(() => {
        state.closed = true;
        for (const running of state.running) {
          try {
            running.kill("SIGKILL");
          } catch {}
        }
      }),
    );
  } catch {}
}
