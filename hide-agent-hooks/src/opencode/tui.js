// Hide's OpenCode 2 plugin, its TUI half, written by Hide's install kit (hide-agent-hooks).
// An edit is kept and shown as edited in Settings; Reinstall puts Hide's back.
// It runs in the terminal, so in a Herdr pane it is that pane: it attaches to the root session the pane shows,
// answers the server half's questions (`server.js`, beside it) with Hide's helper, and keeps the pane's subagent
// counts. Outside a Herdr pane Hide manages, or when its helper is gone, it does nothing.

import { randomUUID } from "node:crypto";

import {
  ANSWER_MARGIN_MS,
  ATTACH_RENEW_MS,
  COUNT_BUDGET_MS,
  RPC,
  SESSION_LIMIT,
  START_BUDGET_MS,
  close,
  helper,
  managed,
  remember,
  withBudget,
} from "./server.js";

const ROUTE_POLL_MS = 250;
const OPERATIONS = new Set(["prompt", "tool", "letters", "confirm", "observation"]);

/**
 * The root session above a child: up the children this half saw start, then as OpenCode's own sessions know it, so
 * a child keeps its root while OpenCode's store lets go of it.
 */
function rootOf(state, api, child) {
  let current = child;
  for (let depth = 0; depth < SESSION_LIMIT && state.children.has(current); depth += 1) current = state.children.get(current).parent;
  return api.data.session.root(current);
}

/**
 * Sends the pane's subagent counts when they changed, or always when `force`. One report runs at a time and only
 * the newest waiting one follows it, so a burst of subagents costs two helper runs, not one per change.
 */
function publishCounts(state, api, force) {
  let working = 0;
  let done = 0;
  for (const [child, status] of state.children) {
    if (rootOf(state, api, child) !== state.root) continue;
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
      const answer = await helper(state, "subagents", { working: next.working, done: next.done }, COUNT_BUDGET_MS, state.pane);
      if (answer?.reported !== true && state.published === next.key) state.published = null;
    }
    state.counting = null;
  })();
  return state.counting;
}

/** At root rest, finished children leave the working count; a background child still running stays. */
function sweep(state, api) {
  for (const [child, status] of state.children) {
    if (status.busy && rootOf(state, api, child) === state.root && api.data.session.status(child) !== "running") status.busy = false;
  }
}

/** Applies one of OpenCode's events to the pane's subagent counts. */
function onEvent(state, api, event) {
  const type = event?.type;
  const data = event?.data ?? {};
  const sessionID = data.sessionID;
  if (typeof sessionID !== "string") return;
  if (type === "session.created") {
    // A child still running is never forgotten: it would leave the working count while it works.
    if (typeof data.parentID === "string") {
      remember(state, state.children, sessionID, { parent: data.parentID, busy: false, ran: false }, [(child) => !child.busy]);
    }
    return;
  }
  const started = type === "session.execution.started";
  if (!started && !["session.execution.succeeded", "session.execution.failed", "session.execution.interrupted"].includes(type)) return;
  const child = state.children.get(sessionID);
  if (child) {
    child.busy = started;
    child.ran ||= started;
    void publishCounts(state, api, false);
  } else if (!started && sessionID === state.root) {
    // Each turn's end reports again, as Claude Code's Stop does, so a Herdr that lost the pane's counts gets them back.
    sweep(state, api);
    void publishCounts(state, api, true);
  }
}

/**
 * The context for a root session's prompt: on the session's first prompt this pane sees, the start guidance and the
 * session-start Memory (with no answer neither is given, and the next prompt asks again); Memory and the letters due.
 * The guidance comes apart from the rest, which the server half leaves out when it carries a letter Hide took.
 */
async function prompt(state, api, input, budgetMs) {
  const deadline = Date.now() + budgetMs;
  const session = input.session_id;
  const first = !state.guided.has(session);
  const answer = await helper(state, "prompt", { ...input, first }, budgetMs, state.pane);
  // After the prompt's own helper, so the report never shares its budget: each turn's start reports again.
  void publishCounts(state, api, true);
  let guidance = "";
  if (first && answer !== null) {
    const start = await withBudget(state.guidance, Math.max(0, deadline - Date.now()));
    if (typeof start?.context === "string") guidance = start.context;
    // Guidance still on its way is given to the next prompt instead.
    if (state.started) remember(state, state.guided, session);
  }
  return {
    guidance,
    context: typeof answer?.context === "string" ? answer.context : "",
    letters: Array.isArray(answer?.letters) ? answer.letters : [],
  };
}

function setup(api) {
  // The terminal's environment is its pane's for as long as it runs.
  const pane = { ...process.env };
  if (!managed(pane)) return;
  const client = randomUUID();
  const rpc = api.client.rpc(RPC);
  const state = {
    pane,
    running: new Set(),
    lost: 0,
    closed: false,
    root: null,
    children: new Map(),
    guided: new Set(),
    started: false,
    guidance: null,
    published: null,
    counting: null,
    nextCount: null,
  };
  state.guidance = helper(state, "start", { cwd: process.cwd() }, START_BUDGET_MS, pane).then((answer) => {
    state.started = true;
    return answer;
  });
  // The root's own place: OpenCode routes a call to the plugin of the folder it names.
  let place;
  const at = (location) => (typeof location?.directory === "string" ? { location: { directory: location.directory } } : undefined);
  const attach = (renew) => {
    if (state.root) rpc.attach({ root: state.root, client, renew }, at(place)).catch(() => {});
  };
  // The pane answers for the root of the session it shows, a subagent's included, and for nothing once it shows
  // none, as on the home screen.
  const select = () => {
    const route = api.ui.router.current();
    const shown = route?.type === "session" ? api.data.session.root(route.sessionID) : null;
    const root = typeof shown === "string" && shown ? shown : null;
    if (root === state.root) return;
    if (state.root) rpc.detach({ root: state.root, client }, at(place)).catch(() => {});
    state.root = root;
    place = root ? api.data.session.get(root)?.location : undefined;
    attach(false);
    void publishCounts(state, api, true);
  };
  const offAsk = rpc.events.on("ask", (event) => {
    const data = event?.data;
    if (!data || !OPERATIONS.has(data.operation) || typeof data.id !== "string") return;
    if (data.client !== client) {
      // A question for any terminal showing the session: a prompt the route has just moved to.
      if (data.client !== null) return;
      select();
    }
    // A question names the session it is for, which may be a subagent's the server half could not place.
    const asked = typeof data.root === "string" ? (api.data.session.root(data.root) ?? data.root) : null;
    if (!state.root || asked !== state.root || !data.input || typeof data.input !== "object") return;
    void (async () => {
      const budget = Math.max(0, Number(data.budget_ms) - ANSWER_MARGIN_MS);
      let output = null;
      try {
        output =
          data.operation === "prompt"
            ? await prompt(state, api, data.input, budget)
            : await helper(state, data.operation, data.input, budget, pane);
      } catch {}
      rpc.answer({ id: data.id, client, output: output ?? {} }, at(event.location)).catch(() => {});
    })();
  });
  const offData = api.data.listen(({ details }) => {
    try {
      onEvent(state, api, details);
    } catch {}
  });
  // The pane's counts start at nothing, sent with the first session it shows or now.
  select();
  if (state.root === null) void publishCounts(state, api, true);
  const poll = setInterval(select, ROUTE_POLL_MS);
  const renew = setInterval(() => attach(true), ATTACH_RENEW_MS);
  return () => {
    clearInterval(poll);
    clearInterval(renew);
    offAsk?.();
    offData?.();
    if (state.root) rpc.detach({ root: state.root, client }, at(place)).catch(() => {});
    close(state);
  };
}

export default { id: "hide.opencode.tui", setup };
