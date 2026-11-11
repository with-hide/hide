// Notifications on the phone (PRD mobile-companion D-19, D-20, D-21, B30,
// B34): permission is asked only when the user turns on the phone switch, the
// subscription is registered over /ws and never over another path, a phone
// that turned notifications back on in the OS subscribes again when the app
// opens, and a revoked phone drops its local subscription.

import { create } from "zustand";
import { translate } from "../i18n/translator";
import { attentionReasonText } from "../missions";
import { MISSION_ATTENTION_REASONS } from "../snapshot";
import { staleTags, type AgentGroup } from "./protocol";
import { patch, usePhone } from "./store";
import { send, currentHost } from "./connection";
import { serviceWorkerAllowed } from "./appHost";
import { hostedPhone } from "./relayRuntime";
import { dropHostPush, hostStore, registerHostPush } from "./hosts";

export function pushSupported(): boolean {
  return serviceWorkerAllowed() && "PushManager" in window && "Notification" in window;
}

export function permission(): NotificationPermission | "unsupported" {
  return "Notification" in window ? Notification.permission : "unsupported";
}

function applicationServerKey(base64url: string): Uint8Array<ArrayBuffer> {
  const base64 = base64url.replace(/-/g, "+").replace(/_/g, "/");
  const binary = atob(base64 + "=".repeat((4 - (base64.length % 4)) % 4));
  const bytes = new Uint8Array(new ArrayBuffer(binary.length));
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
  return bytes;
}

async function registration(host = currentHost()): Promise<ServiceWorkerRegistration | null> {
  if (!serviceWorkerAllowed()) return null;
  try {
    if (hostedPhone()) {
      if (!host || usePhone.getState().relayError === "storage") return null;
      const worker = await registerHostPush(host.room);
      if (worker.active) return worker;
      const pending = worker.installing ?? worker.waiting;
      if (!pending) return null;
      await new Promise<void>((resolve, reject) => {
        const timeout = window.setTimeout(() => { pending.removeEventListener("statechange", changed); reject(new Error("worker_timeout")); }, 15_000);
        const changed = () => {
          if (pending.state !== "activated" && pending.state !== "redundant") return;
          clearTimeout(timeout); pending.removeEventListener("statechange", changed);
          if (pending.state === "activated") resolve(); else reject(new Error("worker_failed"));
        };
        pending.addEventListener("statechange", changed);
        changed();
      });
      return worker;
    }
    return await navigator.serviceWorker.ready;
  } catch {
    return null;
  }
}

/**
 * Gives the service worker the words a notification body starts with, in
 * the language in effect; it keeps them so a closed app still notifies in it.
 */
export async function postWords(worker: ServiceWorkerRegistration | null = null): Promise<void> {
  worker ??= await registration();
  const statusWords = Object.fromEntries((["question", "approval", "error", "working", "done", "idle", "unknown", "waiting", "stopped", "unfinished"] as const).map((code) => [`status_${code}`, translate(`agents.status.${code}`)]));
  // A subject's words keep their `{{count}}` for the worker to fill: an interpolation marker no catalog uses leaves it as written.
  const planReview = translate("plan.askReview", { count: 0, interpolation: { prefix: "\u0000", suffix: "\u0000" } });
  // A mission node raised to the operator is worded by why, the node's title put before it by the worker (PRD mission-node-simple B14).
  const attentionWords = Object.fromEntries(MISSION_ATTENTION_REASONS.map((reason) => [`subject_mission_attention_${reason}`, attentionReasonText(reason, null, translate)]));
  worker?.active?.postMessage({ type: "words", words: { needs_you: translate("mobile.group.needs_you"), done: translate("mobile.group.done"), observer_unconfirmed: translate("mobile.notice.observerUnconfirmed"), letter_undelivered: translate("mobile.notice.letterUndelivered"), subject_plan_review: planReview, subject_mission_decision: translate("mission.askDecision"), subject_mission_attention: translate("mission.attention"), subject_input_unconfirmed_brief: translate("agents.input.brief"), subject_input_unconfirmed_letter: translate("agents.input.letter"), subject_input_unconfirmed_brief_held: translate("agents.input.briefHeld"), subject_input_unconfirmed_letter_held: translate("agents.input.letterHeld"), subject_human_turn: translate("agents.human.mark"), subject_human_turn_auth: translate("agents.human.auth"), subject_human_turn_permission: translate("agents.human.permission"), ...attentionWords, ...statusWords, status_look: translate("agents.mark.look") } });
}

const PREF_KEY = "hide.phone.web-push";

function readPreference(): { enabled: boolean; kept: boolean } {
  // No saved choice keeps the existing permission-based behavior on upgrade.
  try { return { enabled: localStorage.getItem(PREF_KEY) !== "off", kept: true }; }
  catch { return { enabled: true, kept: false }; }
}

export const useBrowserPush = create(() => ({ ...readPreference(), permission: permission(), asking: false, stopping: false, failure: null as "on" | "off" | null }));
const setPush = useBrowserPush.setState;

function choose(enabled: boolean): void {
  let kept = true;
  try { localStorage.setItem(PREF_KEY, enabled ? "on" : "off"); } catch { kept = false; }
  setPush({ enabled, kept, failure: null });
}

function enabled(): boolean {
  try { return localStorage.getItem(PREF_KEY) === "off" ? false : useBrowserPush.getState().enabled; }
  catch { return useBrowserPush.getState().enabled; }
}

/** Re-read browser permission and this browser's choice when Settings becomes visible. */
export function refreshBrowserPush(): void {
  const saved = readPreference();
  setPush({ ...(saved.kept ? saved : { kept: false }), permission: permission() });
}

// One subscription attempt and one coalesced follow-up for a replaced computer, never a queue per meta frame.
let subscribing: { host: ReturnType<typeof currentHost>; key: string | null; result: Promise<boolean> } | null = null;
let followUp = false;

async function unsubscribe(subscription: PushSubscription | null | undefined): Promise<void> {
  if (subscription && !(await subscription.unsubscribe())) throw new Error("unsubscribe_failed");
}

async function subscribe(): Promise<boolean> {
  const host = currentHost();
  const key = usePhone.getState().vapidKey;
  if (subscribing) {
    if (subscribing.host !== host || subscribing.key !== key) followUp = true;
    return subscribing.result;
  }
  const current = () => enabled() && permission() === "granted" && usePhone.getState().connected && currentHost() === host && usePhone.getState().vapidKey === key;
  const result = (async () => {
    let owned: PushSubscription | null = null;
    try {
      const worker = await registration(host);
      if (!key || !worker || !current()) return false;
      await unsubscribe(await worker.pushManager.getSubscription());
      if (!current()) return false;
      owned = await worker.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: applicationServerKey(key) });
      // Off, revoke or a host switch can win while the browser makes the subscription.
      if (!current()) return false;
      const json = owned.toJSON();
      if (!json.endpoint || !json.keys?.p256dh || !json.keys.auth) return false;
      void postWords(worker);
      const sent = send({ type: "push_subscription", subscription: { endpoint: json.endpoint, keys: { p256dh: json.keys.p256dh, auth: json.keys.auth } } });
      if (sent) owned = null;
      return sent;
    } catch {
      return false;
    } finally {
      // Until the registration is sent, every exit still owns the new subscription.
      if (owned) {
        try { await unsubscribe(owned); }
        catch { if (!enabled()) setPush({ failure: "off" }); }
      }
    }
  })();
  subscribing = { host, key, result };
  try {
    const sent = await result;
    if (!sent && current()) setPush({ failure: "on" });
    return sent;
  } finally {
    subscribing = null;
    if (followUp) { followUp = false; void syncPush(); }
  }
}

/** The phone's switch is the only place that asks browser permission. */
export async function enableNotifications(): Promise<void> {
  if (!pushSupported() || useBrowserPush.getState().asking || useBrowserPush.getState().stopping) return;
  setPush({ asking: true, failure: null });
  try {
    const answer = await Notification.requestPermission();
    setPush({ permission: answer });
    if (answer === "granted") {
      choose(true);
      await subscribe();
    } else {
      choose(false);
      send({ type: "push_permission", state: answer === "denied" ? "denied" : "default" });
      patch({ notifications: answer === "denied" ? "off" : "unasked" });
    }
  } catch { setPush({ failure: "on" }); }
  finally { setPush({ asking: false }); }
}

/** Off is kept across reconnects and removes subscriptions for every paired computer in this browser. */
export async function disableNotifications(): Promise<void> {
  if (useBrowserPush.getState().stopping) return;
  choose(false);
  setPush({ stopping: true });
  send({ type: "push_permission", state: "denied" });
  patch({ notifications: "off" });
  try {
    const stop = async (worker: ServiceWorkerRegistration | null | undefined) => {
      await unsubscribe(await worker?.pushManager?.getSubscription());
    };
    const outcomes = hostedPhone()
      ? await Promise.allSettled((await hostStore().list()).map(async (host) => {
        const worker = await navigator.serviceWorker.getRegistration(`/push/${host.room}/`);
        if (worker && new URL(worker.scope).pathname === `/push/${host.room}/`) await stop(worker);
      }))
      : await Promise.allSettled([navigator.serviceWorker.getRegistration("/m/").then(async (worker) => {
        if (worker && new URL(worker.scope).pathname === "/m/") await stop(worker);
      })]);
    if (outcomes.some((outcome) => outcome.status === "rejected")) setPush({ failure: "off" });
  } catch { setPush({ failure: "off" }); }
  finally { setPush({ stopping: false }); }
}

/** Registration refusal is actionable on the switch; a late success cannot undo Off. */
export function browserPushResult(notifications: "on" | "refused"): void {
  if (notifications === "refused") { if (enabled()) setPush({ failure: "on" }); }
  else if (!enabled()) { patch({ notifications: "off" }); send({ type: "push_permission", state: "denied" }); }
  else setPush({ failure: null });
}

/** After each meta: honor this phone's choice even when a computer has an older global mode. */
export async function syncPush(): Promise<void> {
  if (!pushSupported()) return;
  const state = usePhone.getState();
  const current = permission();
  setPush({ permission: current });
  if (!enabled() || current === "denied") {
    if (state.notifications !== "off") send({ type: "push_permission", state: "denied" });
    patch({ notifications: "off" });
  } else if (current === "granted" && state.notifications !== "on") {
    await subscribe();
  }
}

/** A revoked phone keeps no subscription of its own (D-19). */
export async function forgetPush(): Promise<void> {
  if (!serviceWorkerAllowed()) return;
  const host = currentHost();
  if (hostedPhone()) { if (host) await dropHostPush(host.room); return; }
  try {
    const worker = await registration();
    await (await worker?.pushManager.getSubscription())?.unsubscribe();
  } catch {
    // The device already refuses this phone; a subscription left behind is never sent to.
  }
}

/** Closes the notifications of agents no longer waiting or done, once the list is known (D-21). */
export async function closeStaleNotifications(groups: AgentGroup[]): Promise<void> {
  const worker = await registration();
  if (!worker) return;
  try {
    const shown = await worker.getNotifications();
    const stale = new Set(staleTags(shown.map((notification) => notification.tag), groups));
    for (const notification of shown) if (stale.has(notification.tag)) notification.close();
  } catch {
    // getNotifications is missing on some engines; the next push closes them instead.
  }
}
