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
  listedAgents,
  macNameOf,
  missionAnswerFailure,
  parseFragment,
  prFailure,
  refusalOf,
  replyProblem,
  sameKey,
  sleepTarget,
  type AgentKey,
  type DetailTab,
  type DetailView,
  type LookRow,
  type PhoneMessage,
  type PhoneMission,
  type QuickKey,
  type ServerFrame,
} from "./protocol";
import { NO_CHOICE, mayHaveStarted, selectionOf, startFailure, startProblem, type StartChoice } from "./start";
import { confirmOf, holds, prOf } from "./prModel";
import { CLOSED_SHEET, answerSentFor, applyFrame, openPr, patch, usePhone, type StartSheet } from "./store";
import { dropTransfer, onLookTransfer } from "./lookTransfer";
import { forgetPush, syncPush } from "./push";
import { RelaySocket, type PhoneSocket } from "./relaySocket";
import { hostedPhone, PHONE_PROTOCOL, takePairing } from "./relayRuntime";
import { readHosts, saveHost, revokeHost, type PairedHost } from "./hosts";
import type { PairingQr } from "../../../hide-relay-protocol/web";

let relayHost: PairedHost | null = null;
let relayPair: PairingQr | null = null;
export const currentHost = () => relayHost;
function storageFailed(): void { patch({ relayError: "storage" }); }


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
  if (hostedPhone()) {
    try {
      if (relayHost && credential) { relayHost = { ...relayHost, credential }; saveHost(localStorage, relayHost); }
      // A revoked device stays listed as disconnected, without the credential it refused.
      else if (relayHost) { relayHost = { ...relayHost, credential: null }; revokeHost(localStorage, relayHost.room); }
    } catch { storageFailed(); }
    return;
  }
  try {
    if (credential) window.localStorage.setItem(CREDENTIAL_KEY, credential);
    else window.localStorage.removeItem(CREDENTIAL_KEY);
  } catch {
    // A private tab refuses storage; the address still carries the credential.
  }
  const hash = credential ? credentialHash(credential) : "";
  window.history.replaceState(null, "", `${window.location.pathname}${hash}`);
}

let socket: PhoneSocket | null = null;
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
type Watch = { ws: PhoneSocket; ready: boolean; heardAt: number; askedAt: number | null; timer: number | null };
let watch: Watch | null = null;
let pingCounter = 0;

function hidden(): boolean {
  return document.visibilityState === "hidden";
}

function armWatch(w: Watch, ms: number): void {
  if (w.timer !== null) window.clearTimeout(w.timer);
  w.timer = window.setTimeout(() => checkWatch(w), ms);
}

function startWatch(ws: PhoneSocket): void {
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
function lose(ws: PhoneSocket): void {
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
  try { socket.send(JSON.stringify(message)); return true; }
  catch { if (socket) lose(socket); return false; }
}

function scheduleRetry(): void {
  if (stopped || retryTimer !== null) return;
  const delay = RETRY_MS[Math.min(attempt, RETRY_MS.length - 1)];
  attempt += 1;
  retryTimer = window.setTimeout(() => {
    retryTimer = null;
    connect();
  }, hostedPhone() ? delay! * (0.75 + Math.random() * 0.5) : delay);
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
  const ws: PhoneSocket = relayHost ? new RelaySocket(relayHost, () => {
    stopped = true;
    window.location.replace(`/?host=${encodeURIComponent(relayHost!.room)}`);
  }, (reason) => {
    if (["connection_limit", "room_limit", "rate_limit", "daily_limit", "frame_limit"].includes(reason)) patch({ relayError: "busy" });
    else if (reason === "unsupported_version") { stopped = true; patch({ screen: "version" }); }
  }) : new WebSocket(socketUrl());
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
      relayPair = null;
      credential = frame.credential;
      storeCredential(frame.credential);
    }
    if (frame.type === "pong") return;
    if (frame.type === "hello") {
      attempt = 0;
      if (relayHost) {
        relayHost = { ...relayHost, name: frame.mac_name || relayHost.name, lastSeen: Date.now() };
        patch({ relayError: usePhone.getState().relayError === "loadError" ? "loadError" : null });
        storeCredential(credential);
      }
      if (watch) watch.ready = true;
      applyFrame(frame);
      reopenDetail();
      reopenStartSheet();
      reopenPr();
      return;
    }
    if (frame.type === "look_head" || frame.type === "look_chunk" || frame.type === "look_end") {
      onLookTransfer(frame, (requestId, seq) => void send({ type: "look_ack", request_id: requestId, seq }));
      markLookSeen();
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
    if (frame.type === "mission_result") {
      onMissionResult(frame);
      return;
    }
    if (frame.type === "pr_result") {
      onPrResult(frame);
      return;
    }
    if (frame.type === "sleep_result") {
      if (usePhone.getState().sleepAction?.requestId === frame.request_id) clearSleepTimer();
      applyFrame(frame);
      return;
    }
    applyFrame(frame);
    if (frame.type === "look_result" || frame.type === "look_changes") markLookSeen();
    if (frame.type === "paired" && hostedPhone()) {
      try { patch({ installHint: localStorage.getItem("hide.phone.installed-hint") !== "seen" }); }
      catch { storageFailed(); }
    }
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
function settleLoss(ws: PhoneSocket, refused: boolean): void {
  if (socket === ws) socket = null;
  stopWatch();
  const state = usePhone.getState();
  failPendingSleep();
  failLoadingLook();
  if (state.missionAnswer) patch({ missionAnswer: null, missionAnswerError: { mission: state.missionAnswer.mission, notice: missionAnswerFailure("offline") } });
  if (state.pendingInput) patch({ pendingInput: null, inputError: inputFailure("offline") });
  if (state.startSheet.pending) patch({ startSheet: { ...state.startSheet, pending: null, error: startFailure("offline") } });
  if (state.prPending) patch({ prPending: null, prNotice: { key: state.prPending.key, notice: prFailure("offline") } });
  if (state.prDelegate?.pending) patch({ prDelegate: { ...state.prDelegate, pending: null, error: startFailure("offline") } });
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

/** Opens an agent's detail; the eye row of the list opens it on 볼 것 (B11), any other row on the conversation. */
export function openDetail(key: AgentKey, tab: DetailTab = "conversation"): void {
  const state = usePhone.getState();
  if (!state.groups) {
    patch({ pendingOpen: key });
    return;
  }
  const view: DetailView = "conversation";
  dropTransfer();
  patch({ detail: { key, view, tab, rows: null, moreAskedAt: null, conversation: null, olderAskedAt: null, looks: null, look: null }, inputError: null, pendingInput: null });
  send({ type: "open", device_id: key.device_id, pane_id: key.pane_id, view });
}

/**
 * A tab of the detail. Conversation and terminal are what hided reads for the phone, so it is told; 볼 것 is already on its way
 * with the detail and stays the phone's own choice.
 */
export function setTab(tab: DetailTab): void {
  const detail = usePhone.getState().detail;
  if (!detail || (detail.tab === tab && !detail.look)) return;
  if (tab === "looks") {
    patch({ detail: { ...detail, tab } });
    return;
  }
  // Leaving 볼 것 ends a View that is open.
  dropTransfer();
  patch({ detail: { ...detail, tab, view: tab, look: null } });
  if (detail.view !== tab) send({ type: "view", view: tab });
}

export function closeDetail(): void {
  dropTransfer();
  patch({ detail: null, inputError: null, pendingInput: null });
  send({ type: "close" });
}

/** Opens one View full screen and asks hided for its content by id alone (B13, D-16). */
export function openLook(row: LookRow): void {
  const detail = usePhone.getState().detail;
  if (!detail) return;
  const requestId = newRequestId();
  dropTransfer();
  const sent = send({ type: "look_open", request_id: requestId, device_id: detail.key.device_id, pane_id: detail.key.pane_id, view_id: row.id });
  patch({ detail: { ...detail, look: { row, requestId, content: sent ? { state: "loading" } : { state: "failed" }, shown: false, refreshing: false } } });
}

/**
 * A page's screen again (B20, B21): the same View under a new request while the screen on view stays, so a refresh that cannot be
 * taken replaces it with the address and the message and one that is taken replaces it with the new one. Without a connection
 * nothing is sent and nothing changes (B45); another kind of View starts over as a new read.
 */
export function refreshLook(): void {
  const detail = usePhone.getState().detail;
  const look = detail?.look;
  if (!detail || !look) return;
  if (look.content.state !== "shot" && look.content.state !== "page") {
    openLook(look.row);
    return;
  }
  if (look.refreshing) return;
  const requestId = newRequestId();
  if (!send({ type: "look_open", request_id: requestId, device_id: detail.key.device_id, pane_id: detail.key.pane_id, view_id: look.row.id })) return;
  dropTransfer();
  patch({ detail: { ...detail, look: { ...look, requestId, refreshing: true } } });
}

/** "다시": the same View from the start, under a new request, so nothing of the failed read is reused (B23). */
export function retryLook(): void {
  const row = usePhone.getState().detail?.look?.row;
  if (row) openLook(row);
}

/** Back to the list; a read still going is dropped. */
export function closeLook(): void {
  const detail = usePhone.getState().detail;
  if (!detail?.look) return;
  dropTransfer();
  patch({ detail: { ...detail, look: null } });
}

/** A read the lost socket cut off is a failed read, with 다시 (B23). */
function failLoadingLook(): void {
  const detail = usePhone.getState().detail;
  if (detail?.look?.refreshing) {
    // The screen on view stays; the refresh that was cut off is simply not coming.
    dropTransfer();
    patch({ detail: { ...detail, look: { ...detail.look, refreshing: false } } });
    return;
  }
  if (detail?.look?.content.state !== "loading") return;
  dropTransfer();
  patch({ detail: { ...detail, look: { ...detail.look, content: { state: "failed" } } } });
}

/** What the phone drew about a View, however much of it: a loading, closed, failed or unreachable screen is not that (B24, D-15). */
const SEEN_STATES: readonly string[] = ["text", "image", "shot", "changes", "page", "other", "too_large"];

/** The View on screen was opened and the phone drew its answer: it is seen, once, here and on the desktop (B24, D-15). */
function markLookSeen(): void {
  const detail = usePhone.getState().detail;
  const look = detail?.look;
  if (!detail || !look || look.shown || !SEEN_STATES.includes(look.content.state)) return;
  const sent = send({ type: "look_seen", device_id: detail.key.device_id, pane_id: detail.key.pane_id, view_id: look.row.id });
  if (sent) patch({ detail: { ...detail, look: { ...look, shown: true } } });
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

/** What the decision's answer sends: a choice counted from 1, a written line, or the desktop's own "send again" for one typed without confirmation. */
export type MissionAnswerChoice = { option: number } | { text: string } | { resend: true };

/** An answer whose outcome is unknown keeps its request id, so a resend of the same press lands once (B35). */
let lastMissionAnswer: { key: string; requestId: string } | null = null;

/**
 * The operator's answer to a mission's decision: the desktop's `mission_answer`, by request id. The page draws it sent until the core keeps it,
 * refuses it, or asks something else; a refusal the operator can act on comes back on the mission, in the desktop's words.
 */
export function answerMission(mission: PhoneMission, choice: MissionAnswerChoice, words: string): boolean {
  const state = usePhone.getState();
  // One press of this mission at a time; an answer left from another mission's page does not count.
  if (answerSentFor(state.missionAnswer, mission.id)) return false;
  const key = mission.question?.key ?? 0;
  const intent = JSON.stringify([mission.id, key, choice]);
  const requestId = lastMissionAnswer?.key === intent ? lastMissionAnswer.requestId : newRequestId();
  lastMissionAnswer = { key: intent, requestId };
  const sent = send({ type: "answer_mission", request_id: requestId, mission: mission.id, key, ...choice });
  patch(sent
    ? { missionAnswer: { requestId, mission: mission.id, pressed: { key, text: words, failed: mission.answer_failed, resend: "resend" in choice } }, missionAnswerError: null }
    : { missionAnswerError: { mission: mission.id, notice: missionAnswerFailure("offline") } });
  return sent;
}

/** Drops the sent answer's own drawing once the core's state stands for it. */
export function settleMissionAnswer(): void {
  patch({ missionAnswer: null });
}

function onMissionResult(frame: Extract<ServerFrame, { type: "mission_result" }>): void {
  const sent = usePhone.getState().missionAnswer;
  // Only the answer to the press this page is waiting on.
  if (!sent || frame.request_id !== sent.requestId) return;
  if (!frame.ok) {
    // Nothing reached the core: the text stays in its field and the same press may go again.
    lastMissionAnswer = null;
    patch({ missionAnswer: null, missionAnswerError: { mission: sent.mission, notice: missionAnswerFailure(frame.reason) } });
    return;
  }
  lastMissionAnswer = null;
  const drafts = { ...usePhone.getState().missionDrafts };
  delete drafts[sent.mission];
  patch({ missionDrafts: drafts, missionAnswerError: null });
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
  if (!startedAgent || !groups || !listedAgents(groups).some((agent) => sameKey(agent, startedAgent))) return;
  patch({ startedAgent: null });
  openDetail(startedAgent);
}

function reopenStartSheet(): void {
  const state = usePhone.getState();
  if (state.startSheet.open || state.prDelegate) send({ type: "start_sheet", open: true });
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
  if (onDelegateResult(frame)) return;
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

/** Reads the card of the page that is open, so the page and its Merge show what the core read for the head now (B39). */
function readOpenPr(lookAgain = false): void {
  const key = usePhone.getState().prOpen;
  if (!key) return;
  send({ type: "read_pr", request_id: newRequestId(), pr: key, ...(lookAgain ? { look_again: true } : {}) });
}

/** After a lost connection the page's card and the hand-over sheet's catalog and feedback are read again. */
function reopenPr(): void {
  readOpenPr();
  const sheet = usePhone.getState().prDelegate;
  if (sheet) readFeedback(sheet.key);
}

/** Opens a pull request's page and asks the core for its card, as the desktop's card does when it opens (B39). */
export function openPrPage(key: string): void {
  openPr(key);
  readOpenPr();
}

/** The desktop's `Look again` after a write GitHub refused. */
export function lookAgainPr(): void {
  readOpenPr(true);
}

/** Opens the merge confirmation for what the page shows now; nothing in it is preselected (B41). */
export function askMerge(): void {
  const { prs, prOpen } = usePhone.getState();
  const pr = prOf(prs?.items ?? null, prOpen);
  const confirm = pr ? confirmOf(pr) : null;
  if (confirm) patch({ prConfirm: confirm, prNotice: null });
}

export function cancelMerge(): void {
  patch({ prConfirm: null });
}

/** A merge whose outcome is unknown keeps its request id, so a resend of the same press reaches GitHub once (B43). */
let lastMerge: { key: string; requestId: string } | null = null;

/** Sends the confirmed merge, for the head and method the confirmation named, when the page still shows them (B41-B43). */
export function confirmMerge(): boolean {
  const state = usePhone.getState();
  const confirm = state.prConfirm;
  if (!confirm || state.prPending) return false;
  const pr = prOf(state.prs?.items ?? null, confirm.key);
  patch({ prConfirm: null });
  if (!pr || !holds(confirm, pr)) return false;
  const intent = JSON.stringify([confirm.key, confirm.head, confirm.queue ? null : confirm.method]);
  const requestId = lastMerge?.key === intent ? lastMerge.requestId : newRequestId();
  lastMerge = { key: intent, requestId };
  const sent = send({
    type: "merge_pr",
    request_id: requestId,
    pr: confirm.key,
    head: confirm.head,
    ...(confirm.queue || !confirm.method ? {} : { method: confirm.method }),
  });
  patch(sent ? { prPending: { requestId, key: confirm.key }, prNotice: null } : { prNotice: { key: confirm.key, notice: prFailure("offline") } });
  return sent;
}

function onPrResult(frame: Extract<ServerFrame, { type: "pr_result" }>): void {
  const state = usePhone.getState();
  if (frame.action === "merge_pr") {
    const pending = state.prPending;
    // Only the answer to the merge this page is waiting on.
    if (!pending || frame.request_id !== pending.requestId) return;
    // The core's own state, on the card, says what GitHub then did; a refusal that sent nothing may go again under a new id.
    if (frame.ok || (frame.reason !== "in_flight" && frame.reason !== "offline")) lastMerge = null;
    patch({ prPending: null, prNotice: frame.ok ? null : { key: pending.key, notice: prFailure(frame.reason) } });
    return;
  }
  if (frame.ok) return;
  if (frame.action === "read_pr" && frame.pr && frame.pr === state.prOpen) {
    patch({ prNotice: { key: frame.pr, notice: prFailure(frame.reason) } });
    return;
  }
  const sheet = state.prDelegate;
  if (frame.action === "read_pr_feedback" && sheet && sheet.feedbackId === frame.request_id) {
    patch({ prDelegate: { ...sheet, feedbackId: null, error: prFailure(frame.reason) } });
  }
}

/** Asks the core to read the failed checks and change requests a hand-over's prompt is made from. */
function readFeedback(key: string): void {
  const requestId = newRequestId();
  const sheet = usePhone.getState().prDelegate;
  if (!sheet || sheet.key !== key) return;
  patch({ prDelegate: { ...sheet, feedbackId: requestId, error: null } });
  send({ type: "read_pr_feedback", request_id: requestId, pr: key });
}

/** Opens the hand-over sheet; hided sends the agent kinds and what the desktop remembers while it is open (B44). */
export function openDelegate(key: string): void {
  if (usePhone.getState().prDelegate?.key === key) return;
  patch({ prDelegate: { key, prompt: "", edited: false, choice: NO_CHOICE, feedbackId: null, pending: null, error: null }, prDelegated: null });
  send({ type: "start_sheet", open: true });
  readFeedback(key);
}

/** Closes the sheet and, unless the start sheet still needs it, the catalog with it. */
export function closeDelegate(): void {
  patch({ prDelegate: null });
  if (!usePhone.getState().startSheet.open) send({ type: "start_sheet", open: false });
}

export function editDelegate(next: Partial<Pick<PrDelegateDraft, "prompt" | "choice">>): void {
  const sheet = usePhone.getState().prDelegate;
  if (!sheet) return;
  patch({ prDelegate: { ...sheet, ...next, edited: sheet.edited || next.prompt !== undefined, error: null } });
}

/** The prompt a finished feedback read made, put in the field while the operator has not typed. */
export function fillDelegatePrompt(prompt: string): void {
  const sheet = usePhone.getState().prDelegate;
  if (!sheet || sheet.edited || sheet.prompt === prompt) return;
  patch({ prDelegate: { ...sheet, prompt } });
}

export function readFeedbackAgain(): void {
  const sheet = usePhone.getState().prDelegate;
  if (sheet) readFeedback(sheet.key);
}

type PrDelegateDraft = { prompt: string; choice: StartChoice };

/** A hand-over whose outcome is unknown keeps its request id, so a resend of the same one starts one agent (B43 for start). */
let lastDelegate: { key: string; requestId: string } | null = null;

/** Sends the hand-over with the sheet's prompt and the agent kind and model the sheet shows (D-13). */
export function submitDelegate(): boolean {
  const state = usePhone.getState();
  const sheet = state.prDelegate;
  if (!sheet || sheet.pending || !state.startCatalog) return false;
  const problem = startProblem(sheet.prompt);
  if (problem) {
    patch({ prDelegate: { ...sheet, error: startFailure(problem) } });
    return false;
  }
  const selection = selectionOf(state.startCatalog, sheet.choice);
  const key = JSON.stringify([sheet.key, sheet.prompt, selection.kind, selection.model]);
  const requestId = lastDelegate?.key === key ? lastDelegate.requestId : newRequestId();
  lastDelegate = { key, requestId };
  const sent = send({
    type: "delegate_pr",
    request_id: requestId,
    pr: sheet.key,
    prompt: sheet.prompt,
    kind: selection.kind,
    ...(selection.model ? { model: selection.model } : {}),
  });
  patch({ prDelegate: { ...sheet, pending: sent ? requestId : null, error: sent ? null : startFailure("offline") } });
  return sent;
}

/** hided's answer to a hand-over: the sheet closes on a start and keeps its prompt on a refusal. */
function onDelegateResult(frame: Extract<ServerFrame, { type: "start_result" }>): boolean {
  const sheet = usePhone.getState().prDelegate;
  if (!sheet?.pending || frame.request_id !== sheet.pending) return false;
  if (!frame.ok) {
    if (!mayHaveStarted(frame.reason)) lastDelegate = null;
    patch({ prDelegate: { ...sheet, pending: null, error: startFailure(frame.reason) } });
    return true;
  }
  lastDelegate = null;
  patch({ prDelegate: null, prDelegated: sheet.key });
  if (!usePhone.getState().startSheet.open) send({ type: "start_sheet", open: false });
  return true;
}

/** The user tapped Connect on the pairing screen (B11). */
export function pairNow(): void {
  const pair = usePhone.getState().pair;
  if (!pair && !relayPair) return;
  pairCode = relayPair?.code ?? pair!.code;
  stopped = false;
  connect();
}

/** Reads the page address and starts: pair, reconnect, or show how to pair (B11-B13). */
export function start(): void {
  if (hostedPhone()) { void startRelay(); return; }
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


async function startRelay(): Promise<void> {
  try {
    try { relayPair = await takePairing(); }
    catch { patch({ screen: "unpaired", relayError: "qrError" }); return; }
    const query = new URLSearchParams(window.location.search);
    const tag = query.get("open");
    if (tag) {
      const { open } = parseFragment(`#open=${encodeURIComponent(tag)}`);
      if (open) patch({ pendingOpen: open });
    }
    if (relayPair) {
      relayHost = { room: relayPair.room, noiseKey: relayPair.noise_key, name: relayPair.name,
        credential: null, protocol: PHONE_PROTOCOL, lastSeen: null };
      patch({ screen: "pair", macName: relayPair.name });
      if (query.get("connect") === "1") pairNow();
    } else {
      relayHost = readHosts(localStorage).find((host) => host.room === query.get("host")) ?? null;
      if (!relayHost) { window.location.replace("/"); return; }
      credential = relayHost.credential;
      // A revoked device opened from a link or a notification says so without dialing it.
      if (!credential) { stopped = true; patch({ screen: "refused", refusal: "revoked" }); return; }
      patch({ screen: "app", macName: relayHost.name, unreachable: !navigator.onLine });
      connect();
    }
    window.addEventListener("online", wake);
    window.addEventListener("offline", () => { if (socket) lose(socket); });
    document.addEventListener("visibilitychange", () => { if (!hidden()) wake(); });
    window.addEventListener("pagehide", stop);
    window.addEventListener("pageshow", (event) => { if (event.persisted) window.location.reload(); });
  } catch { patch({ screen: "unpaired", relayError: "storage" }); }
}

/** End timers and sockets before a host switch; no pending command crosses hosts. */
export function stop(): void {
  stopped = true;
  if (retryTimer !== null) window.clearTimeout(retryTimer);
  retryTimer = null;
  clearSleepTimer();
  stopWatch();
  if (socket) { socket.onopen = socket.onmessage = socket.onclose = null; socket.close(); socket = null; }
}
