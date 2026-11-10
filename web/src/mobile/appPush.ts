/**
 * The iOS app's notifications as the page lives them (`HidePush`, PRD ios-push). One preference, kept on this phone only;
 * the keys are per computer and made natively. The row in Settings is one switch with a state the person can read:
 *
 * - off: nothing is registered with any computer;
 * - asking: the system's permission question is open;
 * - on: this computer holds a registration and encrypts each notice to the key the phone made for it;
 * - refused: iOS does not allow notifications for the app, and the row says where to turn that on;
 * - failed: the registration could not be made, and the switch stays off to be tried again.
 *
 * Permission is asked only from the person's tap on the switch (`turnOn`). Every connect of a computer sends its
 * registration again (`syncAppPush`), because Apple may change the token and a computer only holds what it was last told.
 */
import { create } from "zustand";
import { translate } from "../i18n/translator";
import { appHost } from "./appHost";
import { nativeCode, push, type PushPermission } from "./appBridge";
import { currentHost, send } from "./connection";
import { hostStore } from "./hosts";
import { usePhone } from "./store";

const PREF_KEY = "hide.phone.app-push";

export type AppPushRow = "off" | "asking" | "on" | "refused" | "failed";

interface AppPushState {
  /** Whether the person turned notifications on; the demo's own switch is `demo`. */
  pref: boolean;
  demo: boolean;
  permission: PushPermission;
  asking: boolean;
  failed: boolean;
  /** The computer refused the registration: it is older than the phone's app. */
  outdated: boolean;
}

export const useAppPush = create<AppPushState>(() => ({ pref: readPref(), demo: false, permission: "unknown", asking: false, failed: false, outdated: false }));
const set = (next: Partial<AppPushState>) => useAppPush.setState(next);

function readPref(): boolean {
  try { return localStorage.getItem(PREF_KEY) === "on"; } catch { return false; }
}

function writePref(on: boolean): void {
  try { if (on) localStorage.setItem(PREF_KEY, "on"); else localStorage.removeItem(PREF_KEY); } catch { /* the choice is kept for this page only */ }
  set({ pref: on });
}

/** The row's state from what is known now. Refused wins over a kept preference: iOS decides, not the page. */
export function appPushRow(state: Pick<AppPushState, "pref" | "demo" | "permission" | "asking" | "failed">, demo: boolean): AppPushRow {
  if (state.asking) return "asking";
  if (state.permission === "denied") return "refused";
  if (state.failed) return "failed";
  if (demo ? state.demo : state.pref && state.permission === "granted") return "on";
  return "off";
}

/** Reads what iOS allows now; called when the Settings tab opens and after a return from iOS Settings. */
export async function refreshAppPush(): Promise<void> {
  if (!appHost()) return;
  try { set({ permission: await push.state() }); } catch { /* an older app bundle has no push plugin: the row stays off */ }
}

async function registerWith(room: string, create: boolean): Promise<boolean> {
  const registration = create ? await push.enable(room) : await push.registration(room);
  if (!registration) return false;
  return send({ type: "apns_registration", token: registration.token, environment: registration.environment, key: { p256dh: registration.p256dh, auth: registration.auth } });
}

/** After every `meta` of a connected computer: register again, or unregister when the person turned notifications off. */
export async function syncAppPush(): Promise<void> {
  if (!appHost() || usePhone.getState().demo) return;
  const room = currentHost()?.room;
  if (!room) return;
  try {
    const permission = await push.state();
    set({ permission });
    const registered = usePhone.getState().notifications === "on";
    if (!readPref() || permission === "denied") {
      if (registered) send({ type: "push_permission", state: "denied" });
      if (!readPref()) await push.forget(room);
      return;
    }
    // Permission is granted here, so this asks nothing: it makes the computer's key on its first connect after the
    // switch was turned on elsewhere.
    set({ outdated: false });
    await registerWith(room, true);
  } catch (error) {
    // A failure the person cannot act on goes to the log; the row says only what is true (principle 13).
    console.warn("phone.app_push_sync_failed", { code: nativeCode(error) });
  }
}

/** The computer answered `refused_request` to the registration: its hided does not know the message yet. */
export function appPushRefused(): void {
  set({ outdated: true });
}

/** The switch turned on. In the demo it posts the sample notification, made as a real one is for the first sample agent's question: the agent's title, then the status word, the question and the project; otherwise it asks iOS, makes this computer's key and registers. */
export async function turnOnAppPush(): Promise<void> {
  if (useAppPush.getState().asking) return;
  set({ asking: true, failed: false });
  try {
    if (usePhone.getState().demo) {
      await push.demoSample({ title: translate("mobile.demo.a1.title"), status_code: "question", state: "needs_you", detail: translate("mobile.demo.a1.ask"), place: "demo · fix/review" });
      set({ demo: true, permission: "granted" });
      return;
    }
    const room = currentHost()?.room;
    if (!room) throw new Error("no_computer");
    const registration = await push.enable(room);
    writePref(true);
    set({ permission: "granted", outdated: false });
    send({ type: "apns_registration", token: registration.token, environment: registration.environment, key: { p256dh: registration.p256dh, auth: registration.auth } });
  } catch (error) {
    if (nativeCode(error) === "denied") set({ permission: "denied" });
    else set({ failed: true });
  } finally {
    set({ asking: false });
  }
}

/**
 * The switch turned off. The connected computer is told first and every key is then deleted, the connected one even when
 * the send could not go out; a computer that is not connected is unregistered on its next connect (`syncAppPush`).
 */
export async function turnOffAppPush(): Promise<void> {
  set({ failed: false });
  if (usePhone.getState().demo) { set({ demo: false }); return; }
  writePref(false);
  send({ type: "push_permission", state: "denied" });
  try {
    for (const host of await hostStore().list()) await push.forget(host.room);
  } catch (error) {
    console.warn("phone.app_push_forget_failed", { code: nativeCode(error) });
  }
}

/** A computer is removed: unregister it when it is the one connected, then delete its key. */
export async function leaveHostPush(room: string): Promise<void> {
  if (!appHost()) return;
  if (currentHost()?.room === room) send({ type: "push_permission", state: "denied" });
  try { await push.forget(room); } catch (error) { console.warn("phone.app_push_forget_failed", { code: nativeCode(error) }); }
}
