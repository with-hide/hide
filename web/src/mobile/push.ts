// Notifications on the phone (PRD mobile-companion D-19, D-20, D-21, B30,
// B34): permission is asked only when the user taps Turn on notifications, the
// subscription is registered over /ws and never over another path, a phone
// that turned notifications back on in the OS subscribes again when the app
// opens, and a revoked phone drops its local subscription.

import { translate } from "../i18n/translator";
import { staleTags, type AgentGroup } from "./protocol";
import { patch, usePhone } from "./store";
import { send } from "./connection";

export function pushSupported(): boolean {
  return "serviceWorker" in navigator && "PushManager" in window && "Notification" in window;
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

async function registration(): Promise<ServiceWorkerRegistration | null> {
  if (!("serviceWorker" in navigator)) return null;
  try {
    return await navigator.serviceWorker.ready;
  } catch {
    return null;
  }
}

/**
 * Gives the service worker the words a notification body starts with, in
 * the language in effect; it keeps them so a closed app still notifies in it.
 */
export async function postWords(): Promise<void> {
  const worker = await registration();
  const statusWords = Object.fromEntries((["question", "approval", "error", "working", "done", "idle", "unknown", "waiting", "stopped", "unfinished"] as const).map((code) => [`status_${code}`, translate(`agents.status.${code}`)]));
  // A subject's words keep their `{{count}}` for the worker to fill: an interpolation marker no catalog uses leaves it as written.
  const planReview = translate("plan.askReview", { count: 0, interpolation: { prefix: "\u0000", suffix: "\u0000" } });
  worker?.active?.postMessage({ type: "words", words: { needs_you: translate("mobile.group.needs_you"), done: translate("mobile.group.done"), observer_unconfirmed: translate("mobile.notice.observerUnconfirmed"), letter_undelivered: translate("mobile.notice.letterUndelivered"), subject_plan_review: planReview, subject_mission_decision: translate("mission.askDecision"), ...statusWords } });
}

async function subscribe(): Promise<boolean> {
  const key = usePhone.getState().vapidKey;
  const worker = await registration();
  if (!key || !worker) return false;
  try {
    // hided holds no subscription whenever this runs, so the one the browser
    // kept is either unknown to hided or one the push service already
    // refused (hided drops a subscription on 404 or 410): start a fresh one.
    await (await worker.pushManager.getSubscription())?.unsubscribe();
    const subscription = await worker.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: applicationServerKey(key) });
    void postWords();
    const json = subscription.toJSON();
    if (!json.endpoint || !json.keys?.p256dh || !json.keys.auth) return false;
    return send({ type: "push_subscription", subscription: { endpoint: json.endpoint, keys: { p256dh: json.keys.p256dh, auth: json.keys.auth } } });
  } catch {
    return false;
  }
}

/** The user tapped Turn on notifications (B30). */
export async function enableNotifications(): Promise<void> {
  if (!pushSupported()) return;
  const answer = await Notification.requestPermission();
  if (answer === "granted") {
    if (!(await subscribe())) patch({ notifications: "unasked" });
    return;
  }
  send({ type: "push_permission", state: answer === "denied" ? "denied" : "default" });
  patch({ notifications: answer === "denied" ? "off" : "unasked" });
}

/**
 * After every hello: re-subscribe when the OS allows notifications but hided
 * holds no subscription (the OS setting came back on, or the push service
 * dropped the old one), and tell hided when the OS setting went off (B30, B35).
 */
export async function syncPush(): Promise<void> {
  const state = usePhone.getState();
  if (!pushSupported()) return;
  const current = permission();
  if (current === "granted" && state.notifications !== "on" && state.pushMode !== "off") {
    await subscribe();
  } else if (current === "denied" && state.notifications !== "off") {
    send({ type: "push_permission", state: "denied" });
    patch({ notifications: "off" });
  }
}

/** A revoked phone keeps no subscription of its own (D-19). */
export async function forgetPush(): Promise<void> {
  const worker = await registration();
  try {
    await (await worker?.pushManager.getSubscription())?.unsubscribe();
  } catch {
    // Nothing is sent to a revoked phone anyway; hided dropped its copy in the same write.
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
