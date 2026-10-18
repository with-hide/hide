// The phone's one socket to hided (PRD mobile-companion D-16): pair with the
// QR's code or authenticate with the stored credential, then keep the list
// live and retry forever after a loss (B23). Nothing here calls any HTTP path
// but `/ws`; the page, its script and the service worker are the static shell.
//
// A Mac that went to sleep leaves the socket open on the phone, and hided's own
// ping is invisible to the page, so the phone watches its socket itself: one
// timer per connection, below.

import {
  MAX_MESSAGES,
  credentialHash,
  inputFailure,
  macNameOf,
  parseFragment,
  refusalOf,
  replyProblem,
  sameKey,
  sleepTarget,
  type AgentKey,
  type DetailView,
  type PhoneMessage,
  type QuickKey,
  type ServerFrame,
} from "./protocol";
import { mayHaveStarted, selectionOf, startFailure, startProblem } from "./start";
import { CLOSED_SHEET, applyFrame, patch, usePhone, type StartSheet } from "./store";
import { forgetPush, syncPush } from "./push";

const CREDENTIAL_KEY = "hide.phone.credential";
const RETRY_MS = [1000, 2000, 4000, 8000, 10000];
/** Silence on a live socket before the phone asks hided whether it is there. */
const IDLE_MS = 20_000;
/** How long hided has to answer that question (or send anything) before the socket is given up. */
const REPLY_MS = 10_000;
/** How long a new socket has to say hello; the browser's own connect timeout can take minutes. */
const OPEN_MS = 15_000;

function storedCredential(): string | null {
  try {
    return window.localStorage.getItem(CREDENTIAL_KEY);
  } catch {
    return null;
  }
}

function storeCredential(credential: string | null): void {
  try {
    if (credential) window.localStorage.setItem(CREDENTIAL_KEY, credential);
    else window.localStorage.removeItem(CREDENTIAL_KEY);
  } catch {
    // A private tab refuses storage; the address still carries the credential.
  }
  const hash = credential ? credentialHash(credential) : "";
  window.history.replaceState(null, "", `${window.location.pathname}${hash}`);
}

let socket: WebSocket | null = null;
let credential: string | null = null;
let pairCode: string | null = null;
let attempt = 0;
let retryTimer: number | null = null;
/** Set when hided refused the phone for good: no retry until the page reloads. */
let stopped = false;
/** At most one sleep command from this phone waits for a dispatch answer. */
let sleepTimer: number | null = null;

function clearSleepTimer(): void {
  if (sleepTimer !== null) window.clearTimeout(sleepTimer);
  sleepTimer = null;
}

function failPendingSleep(): void {
  clearSleepTimer();
  const pending = usePhone.getState().sleepAction;
  if (pending?.requestId) patch({ sleepAction: { ...pending, requestId: null, error: true } });
}

/**
 * What the phone knows of one socket's health. Every frame from hided counts as
 * proof it is there, so a busy conversation never pings; after `IDLE_MS` of
 * silence the phone sends `ping` and gives hided `REPLY_MS` to send anything.
 * The longest a silent hided stays shown as connected is therefore
 * `IDLE_MS + REPLY_MS` after its last frame, and `REPLY_MS` after the phone
 * comes back to the foreground. One timer, re-armed for what is left of the
 * wait, so a frame costs one timestamp and no timer.
 */
type Watch = { ws: WebSocket; ready: boolean; heardAt: number; askedAt: number | null; timer: number | null };
let watch: Watch | null = null;
let pingCounter = 0;

function hidden(): boolean {
  return document.visibilityState === "hidden";
}

function armWatch(w: Watch, ms: number): void {
  if (w.timer !== null) window.clearTimeout(w.timer);
  w.timer = window.setTimeout(() => checkWatch(w), ms);
}

function startWatch(ws: WebSocket): void {
  stopWatch();
  const w: Watch = { ws, ready: false, heardAt: Date.now(), askedAt: null, timer: null };
  watch = w;
  armWatch(w, OPEN_MS);
}

function stopWatch(): void {
  if (watch?.timer != null) window.clearTimeout(watch.timer);
  watch = null;
}

function askHided(w: Watch): void {
  pingCounter += 1;
  w.askedAt = Date.now();
  try {
    w.ws.send(JSON.stringify({ type: "ping", id: pingCounter } satisfies PhoneMessage));
  } catch {
    lose(w.ws);
    return;
  }
  armWatch(w, REPLY_MS);
}

function checkWatch(w: Watch): void {
  if (watch !== w) return;
  w.timer = null;
  // A hidden page may have been suspended; its silence proves nothing, and the way back asks again.
  if (hidden()) {
    armWatch(w, IDLE_MS);
    return;
  }
  const now = Date.now();
  if (w.askedAt !== null && w.heardAt >= w.askedAt) w.askedAt = null;
  if (w.askedAt !== null) {
    const left = REPLY_MS - (now - w.askedAt);
    if (left > 0) armWatch(w, left);
    else lose(w.ws);
    return;
  }
  const limit = w.ready ? IDLE_MS : OPEN_MS;
  const quiet = now - w.heardAt;
  if (quiet < limit) armWatch(w, limit - quiet);
  else if (w.ready) askHided(w);
  else lose(w.ws);
}

/** Gives up a socket hided no longer answers, without waiting for the browser to close it. */
function lose(ws: WebSocket): void {
  if (socket !== ws) return;
  ws.onopen = ws.onmessage = ws.onclose = null;
  try {
    ws.close();
  } catch {
    // Already closing.
  }
  settleLoss(ws, false);
}

/** The phone came back (foreground, network): check at once instead of trusting the last state. */
function wake(): void {
  if (stopped) return;
  if (!watch) {
    retryNow();
  } else if (watch.ready) {
    askHided(watch);
  } else {
    watch.heardAt = Date.now();
  }
}

function socketUrl(): string {
  const scheme = window.location.protocol === "https:" ? "wss:" : "ws:";
  return `${scheme}//${window.location.host}/ws`;
}

export function send(message: PhoneMessage): boolean {
  if (!socket || socket.readyState !== WebSocket.OPEN || !usePhone.getState().connected) return false;
  socket.send(JSON.stringify(message));
  return true;
}

function scheduleRetry(): void {
  if (stopped || retryTimer !== null) return;
  const delay = RETRY_MS[Math.min(attempt, RETRY_MS.length - 1)];
  attempt += 1;
  retryTimer = window.setTimeout(() => {
    retryTimer = null;
    connect();
  }, delay);
}

/** Retries at once when the phone comes back (network, foreground). */
function retryNow(): void {
  if (stopped || usePhone.getState().connected) return;
  if (retryTimer !== null) {
    window.clearTimeout(retryTimer);
    retryTimer = null;
  }
  attempt = 0;
  connect();
}

function connect(): void {
  if (stopped) return;
  if (socket && socket.readyState <= WebSocket.OPEN) return;
  // Nothing to present: the unpaired screen stays, no socket is opened.
  if (!pairCode && !credential) return;
  const first = pairCode ? { client_kind: "phone", pair: pairCode } : { client_kind: "phone", token: credential };
  let refused = false;
  const ws = new WebSocket(socketUrl());
  socket = ws;
  startWatch(ws);
  ws.onopen = () => ws.send(JSON.stringify(first));
  ws.onmessage = (event) => {
    if (watch) watch.heardAt = Date.now();
    let frame: ServerFrame;
    try {
      frame = JSON.parse(String(event.data)) as ServerFrame;
    } catch {
      return;
    }
    if (frame.type === "refused") {
      refused = true;
      onRefused(frame.reason);
      return;
    }
    if (frame.type === "paired") {
      pairCode = null;
      credential = frame.credential;
      storeCredential(frame.credential);
    }
    if (frame.type === "pong") return;
    if (frame.type === "hello") {
      attempt = 0;
      if (watch) watch.ready = true;
      applyFrame(frame);
      reopenDetail();
      reopenStartSheet();
      return;
    }
    if (frame.type === "input_result") {
      onInputResult(frame.ok, frame.reason);
      return;
    }
    if (frame.type === "start_result") {
      onStartResult(frame);
      return;
    }
    if (frame.type === "sleep_result") {
      if (usePhone.getState().sleepAction?.requestId === frame.request_id) clearSleepTimer();
      applyFrame(frame);
      return;
    }
    applyFrame(frame);
    if (frame.type === "agents") {
      openPending();
      openStarted();
    }
    // hided sends the push mode right after hello; the subscription follows it.
    if (frame.type === "meta") void syncPush();
  };
  ws.onclose = () => settleLoss(ws, refused);
}

/** The socket is gone, closed or given up: the screen says so and the retries go on (B23). */
function settleLoss(ws: WebSocket, refused: boolean): void {
  if (socket === ws) socket = null;
  stopWatch();
  const state = usePhone.getState();
  failPendingSleep();
  if (state.pendingInput) patch({ pendingInput: null, inputError: inputFailure("offline") });
  if (state.startSheet.pending) patch({ startSheet: { ...state.startSheet, pending: null, error: startFailure("offline") } });
  patch({ connected: false, unreachable: !refused || state.refusal === "mobile_off" });
  if (!refused || state.refusal === "mobile_off") scheduleRetry();
}

function onRefused(reason: string): void {
  failPendingSleep();
  const refusal = refusalOf(reason);
  if (refusal === "mobile_off") {
    // Mobile was switched off on the Mac: the phone stays paired and keeps retrying (B7).
    patch({ refusal: "mobile_off", connected: false, unreachable: true });
    return;
  }
  stopped = true;
  stopWatch();
  if (refusal === "revoked") {
    credential = null;
    storeCredential(null);
    void forgetPush();
  }
  patch({ screen: "refused", refusal, connected: false, unreachable: false, pair: null });
}

function openPending(): void {
  const pending = usePhone.getState().pendingOpen;
  if (!pending) return;
  patch({ pendingOpen: null });
  openDetail(pending);
}

function reopenDetail(): void {
  const detail = usePhone.getState().detail;
  if (!detail) return;
  // A new detail on hided starts the conversation over from its newest page.
  patch({ detail: { ...detail, olderAskedAt: null, moreAskedAt: null } });
  send({ type: "open", device_id: detail.key.device_id, pane_id: detail.key.pane_id, view: detail.view });
}

export function openDetail(key: AgentKey): void {
  const state = usePhone.getState();
  if (!state.groups) {
    patch({ pendingOpen: key });
    return;
  }
  const view: DetailView = "conversation";
  patch({ detail: { key, view, rows: null, moreAskedAt: null, conversation: null, olderAskedAt: null }, inputError: null, pendingInput: null });
  send({ type: "open", device_id: key.device_id, pane_id: key.pane_id, view });
}

/** Conversation or terminal: hided reads only what the phone shows. */
export function setView(view: DetailView): void {
  const detail = usePhone.getState().detail;
  if (!detail || detail.view === view) return;
  patch({ detail: { ...detail, view } });
  send({ type: "view", view });
}

export function closeDetail(): void {
  patch({ detail: null, inputError: null, pendingInput: null });
  send({ type: "close" });
}

/** Asks for the next page of older rows once per page (B25). */
export function loadMore(): void {
  const detail = usePhone.getState().detail;
  if (!detail?.rows?.more || detail.moreAskedAt === detail.rows.lines) return;
  patch({ detail: { ...detail, moreAskedAt: detail.rows.lines } });
  send({ type: "more" });
}

/** Asks for the page of messages before the oldest one held, once per page and up to MAX_MESSAGES. */
export function loadOlder(): void {
  const detail = usePhone.getState().detail;
  const conversation = detail?.conversation;
  if (!detail || !conversation || conversation === "none") return;
  const before = conversation.before;
  if (before === null || conversation.messages.length >= MAX_MESSAGES || detail.olderAskedAt === before) return;
  patch({ detail: { ...detail, olderAskedAt: before } });
  send({ type: "older", before });
}

let requestCounter = 0;
/** A reply that failed keeps its request id, so a resend of the same text lands once (B27). */
let lastReply: { text: string; requestId: string } | null = null;

function newRequestId(): string {
  requestCounter += 1;
  const random = new Uint8Array(8);
  crypto.getRandomValues(random);
  return `${Date.now().toString(36)}-${requestCounter}-${Array.from(random, (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
}

/** One sleep command at a time; its answer or the timer settles the pending state. */
function sendSleepCommand(target: string, requestId: string, message: PhoneMessage): void {
  if (usePhone.getState().sleepAction?.requestId) return;
  const sent = send(message);
  patch({ sleepAction: { sleepId: target, requestId: sent ? requestId : null, error: !sent } });
  if (sent) sleepTimer = window.setTimeout(failPendingSleep, 10_000);
}

/** A retry keeps the SleepId; a lost answer never triggers an automatic resend. */
export function actOnSleepingSession(sleepId: string, type: "wake_sleeping_session" | "check_sleeping_session"): void {
  const requestId = newRequestId();
  sendSleepCommand(sleepId, requestId, { type, sleep_id: sleepId, request_id: requestId });
}

/** Wake agent, Retry and Start new session for a pane whose agent sleeps: the desktop's own wake, in the same pane. */
export function wakeAgent(key: AgentKey, fresh = false): void {
  const requestId = newRequestId();
  sendSleepCommand(sleepTarget(key), requestId, { type: "wake_agent", request_id: requestId, device_id: key.device_id, pane_id: key.pane_id, ...(fresh ? { fresh } : {}) });
}

/** Sends a reply with Enter after it; the caller clears its field only on success (B26, B27). */
export function sendReply(text: string): boolean {
  const state = usePhone.getState();
  const detail = state.detail;
  if (!detail || state.pendingInput) return false;
  const problem = replyProblem(text);
  if (problem) {
    patch({ inputError: inputFailure(problem) });
    return false;
  }
  const requestId = lastReply?.text === text ? lastReply.requestId : newRequestId();
  lastReply = { text, requestId };
  const sent = send({ type: "input", request_id: requestId, device_id: detail.key.device_id, pane_id: detail.key.pane_id, text });
  patch(sent ? { pendingInput: { requestId, kind: "text" }, inputError: null } : { inputError: inputFailure("offline") });
  return sent;
}

export function sendKey(key: QuickKey): void {
  const state = usePhone.getState();
  const detail = state.detail;
  if (!detail || state.pendingInput) return;
  const requestId = newRequestId();
  const sent = send({ type: "input", request_id: requestId, device_id: detail.key.device_id, pane_id: detail.key.pane_id, key });
  patch(sent ? { pendingInput: { requestId, kind: "key" }, inputError: null } : { inputError: inputFailure("offline") });
}

type ReplyListener = (ok: boolean) => void;
const replyListeners = new Set<ReplyListener>();

/** The reply bar clears its field when its reply landed. */
export function onReplyResult(listener: ReplyListener): () => void {
  replyListeners.add(listener);
  return () => replyListeners.delete(listener);
}

function onInputResult(ok: boolean, reason: string | null): void {
  const pending = usePhone.getState().pendingInput;
  patch({ pendingInput: null, inputError: ok ? null : inputFailure(reason) });
  if (pending?.kind === "text") {
    if (ok) lastReply = null;
    for (const listener of replyListeners) listener(ok);
  }
}

/** How long a started agent may take to appear in the list before its detail opens anyway. */
const STARTED_WAIT_MS = 15000;

/** A started agent's detail opens once the list has it, so it never opens as "gone". */
function openStarted(): void {
  const { startedAgent, groups } = usePhone.getState();
  if (!startedAgent || !groups?.some((group) => group.agents.some((agent) => sameKey(agent, startedAgent)))) return;
  patch({ startedAgent: null });
  openDetail(startedAgent);
}

function reopenStartSheet(): void {
  if (usePhone.getState().startSheet.open) send({ type: "start_sheet", open: true });
}

/** Opens the sheet; hided reads the model catalog while it is open and sends what it lists. */
export function openStartSheet(): void {
  const state = usePhone.getState();
  patch({ startSheet: { ...state.startSheet, open: true, error: null } });
  send({ type: "start_sheet", open: true });
}

/** Closes the sheet and keeps its text. */
export function closeStartSheet(): void {
  const state = usePhone.getState();
  patch({ startSheet: { ...state.startSheet, open: false, error: null } });
  send({ type: "start_sheet", open: false });
}

export function editStartSheet(next: Partial<Pick<StartSheet, "text" | "choice">>): void {
  const state = usePhone.getState();
  patch({ startSheet: { ...state.startSheet, ...next, error: null } });
}

/** A start whose outcome is unknown keeps its request id, so a resend of the same start lands once (B43). */
let lastStart: { key: string; requestId: string } | null = null;

/** Sends the sheet's start; the sheet keeps its text until hided says an agent started (B43, B44). */
export function submitStart(): boolean {
  const state = usePhone.getState();
  const sheet = state.startSheet;
  if (sheet.pending) return false;
  const problem = startProblem(sheet.text);
  const selection = selectionOf(state.startCatalog, sheet.choice);
  if (problem || !selection.target) {
    patch({ startSheet: { ...sheet, error: startFailure(problem ?? "unknown_target") } });
    return false;
  }
  const key = JSON.stringify([sheet.text, selection.target.id, selection.kind, selection.model]);
  const requestId = lastStart?.key === key ? lastStart.requestId : newRequestId();
  lastStart = { key, requestId };
  const sent = send({
    type: "start_agent",
    request_id: requestId,
    text: sheet.text,
    target: selection.target.id,
    kind: selection.kind,
    ...(selection.model ? { model: selection.model } : {}),
  });
  patch({ startSheet: { ...sheet, pending: sent ? requestId : null, error: sent ? null : startFailure("offline") } });
  return sent;
}

function onStartResult(frame: Extract<ServerFrame, { type: "start_result" }>): void {
  const sheet = usePhone.getState().startSheet;
  // Only the answer to the start this sheet is waiting on.
  if (!sheet.pending || frame.request_id !== sheet.pending) return;
  if (!frame.ok || !frame.device_id || !frame.pane_id) {
    if (!mayHaveStarted(frame.reason)) lastStart = null;
    patch({ startSheet: { ...sheet, pending: null, error: startFailure(frame.reason) } });
    return;
  }
  lastStart = null;
  const key: AgentKey = { device_id: frame.device_id, pane_id: frame.pane_id };
  patch({ startSheet: CLOSED_SHEET, startedAgent: key });
  send({ type: "start_sheet", open: false });
  openStarted();
  window.setTimeout(() => {
    if (!sameKey(usePhone.getState().startedAgent, key)) return;
    patch({ startedAgent: null });
    openDetail(key);
  }, STARTED_WAIT_MS);
}

/** The user tapped Connect on the pairing screen (B11). */
export function pairNow(): void {
  const pair = usePhone.getState().pair;
  if (!pair) return;
  pairCode = pair.code;
  stopped = false;
  connect();
}

/** Reads the page address and starts: pair, reconnect, or show how to pair (B11-B13). */
export function start(): void {
  const fragment = parseFragment(window.location.hash);
  window.addEventListener("online", wake);
  // A socket outlives a lost network until a ping goes unanswered; the phone
  // knows sooner, so the list and an open start sheet say so at once (B44).
  window.addEventListener("offline", () => {
    if (socket) lose(socket);
  });
  document.addEventListener("visibilitychange", () => {
    if (!hidden()) wake();
  });
  if (fragment.open) patch({ pendingOpen: fragment.open });
  if (fragment.pair) {
    // The code stays out of storage and out of the address once read.
    window.history.replaceState(null, "", window.location.pathname);
    patch({ screen: "pair", pair: fragment.pair, macName: macNameOf(fragment.pair.endpoint) });
    return;
  }
  credential = fragment.credential ?? storedCredential();
  if (!credential) {
    patch({ screen: "unpaired" });
    return;
  }
  storeCredential(credential);
  patch({ screen: "app", unreachable: false });
  // Opened without a network, the list is not blank: the unreachable line shows at once (B23).
  if (!navigator.onLine) patch({ unreachable: true });
  connect();
}
