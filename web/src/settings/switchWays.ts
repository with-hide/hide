// The rules of a device row opened on how its device is connected (operator
// decision 2026-11-24, Doop canvas 1N77v1reLZ): what the machine said of the
// rule every switch passes, whether 서버로 연결 can be pressed for it, where
// a running switch stands, and what a press that did not switch it says.
// hided reads the machine and decides (`pairing_ssh::Condition`); this only
// words what it answered.

import { useCallback, useEffect, useRef, useState } from "react";
import type { Actions } from "../actions";
import type { MessageKey } from "../i18n/catalogs";
import { refusalKey, type PairingResult, type SwitchCondition } from "../pairing";
import type { DeviceSwitchStep } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";

/** A switch's steps in the order hided runs them, each one a line of the progress list. */
export const SWITCH_STEPS: readonly DeviceSwitchStep[] = ["checking", "starting", "pairing", "connecting", "projects"];

/** Where one line of the progress list stands while the switch is at `current`. */
export function stepState(step: DeviceSwitchStep, current: DeviceSwitchStep): "done" | "on" | "later" {
  const [at, now] = [SWITCH_STEPS.indexOf(step), SWITCH_STEPS.indexOf(current)];
  return at < now ? "done" : at === now ? "on" : "later";
}

/**
 * Whether 서버로 연결 can be pressed for a machine in `condition`: pressed
 * (a Mac without automatic login is switched with a warning, and a Linux
 * account that does not linger is asked to on the press), drawn and off
 * (the one thing to change is named beside it), or not drawn at all.
 */
export function switchButton(condition: SwitchCondition): "on" | "off" | "none" {
  switch (condition.kind) {
    case "auto_login_on":
    case "auto_login_off":
    case "linger_on":
    case "linger_off":
      return "on";
    case "signed_out":
    case "linger_admin":
      return "off";
    case "no_systemd":
    case "windows":
    case "unknown":
      return "none";
  }
}

/** What an opened row knows of its machine: nothing asked, a read under way, what it said, or why it could not be read. */
export type SwitchLook = { state: "reading" } | { state: "read"; condition: SwitchCondition; user: string | null } | { state: "unread"; reason: string };

/** How a press that did not switch the device ended: another computer switched it first, or a step failed. */
export type SwitchOutcome = { kind: "held"; by: string; terminals: number | null } | { kind: "failed"; reason: string };

/** The one line a failed press says, by the step that stopped. */
export function switchFailureKey(reason: string): MessageKey {
  switch (reason) {
    case "ssh_core_not_started":
    case "ssh_hide_not_running":
      return "devices.ways.failed.start";
    case "switch_not_connected":
    case "switch_not_moved":
      return "devices.ways.failed.connect";
    case "ssh_mac_from_mac_only":
      return "devices.ways.failed.macOnly";
    default:
      return refusalKey(reason) ?? "devices.ways.failed.other";
  }
}

/** A login that asked for an answer, or a distribution WSL did not open: the machine was not read, and no switch reaches it. */
const NOT_REACHED = new Set(["ssh_not_reached", "wsl_not_reached"]);

/**
 * How long a press waits for its answer: the daemon's own bound on a whole
 * switch (`SWITCH_BOUND` in `hided/src/device_cli.rs`, 380 s) and a little
 * over, so it ends only a press whose answer was lost, never a switch that
 * still runs.
 */
export const SWITCH_ANSWER_WITHIN_MS = 400_000;

/**
 * What a `look_switch` answer, or a press that left the device, says of the
 * machine. A look changes nothing there, so it cannot tell an account Hide
 * may turn lingering on for from one only an administrator may: once a
 * press learned the latter (`admin`), a look that still reads linger off
 * keeps that answer and its command.
 */
function lookOf(result: PairingResult, admin: { current: string | null }): SwitchLook {
  if (!result.ok || !result.condition) return { state: "unread", reason: result.reason ?? "" };
  let condition = result.condition;
  if (condition.kind === "linger_admin") admin.current = condition.command;
  else if (condition.kind === "linger_off" && admin.current !== null) condition = { kind: "linger_admin", command: admin.current };
  else admin.current = null;
  return { state: "read", condition, user: result.user ?? null };
}

/**
 * One device's read and switch, for its row. While `wanted`, the machine is
 * read once over its own login, again each time the row is opened, and on
 * `read`; what was read stays as it is while the row is open. `press` asks
 * for the switch, which reads the machine again first: a machine it leaves
 * answers with what it said, which replaces the look, and any other end
 * that did not switch the device is kept as its outcome (`ui.ts`).
 */
export function useDeviceSwitch(deviceId: string, actions: Actions, wanted: boolean, open: boolean): { look: SwitchLook | null; pressed: boolean; read: () => void; press: () => void } {
  const [look, setLook] = useState<SwitchLook | null>(null);
  const [pressed, setPressed] = useState(false);
  const asks = useRef<{ look: string | null; press: string | null }>({ look: null, press: null });
  const admin = useRef<string | null>(null);
  const deadline = useRef<ReturnType<typeof setTimeout> | null>(null);
  // The press is over, answered or not: its place draws the button or its line again.
  const settle = useCallback(() => {
    if (deadline.current !== null) clearTimeout(deadline.current);
    deadline.current = null;
    asks.current.press = null;
    setPressed(false);
  }, []);
  // A press nothing answered ends as one that failed, to be tried again.
  const unanswered = useCallback(() => {
    settle();
    useUiStore.getState().setSwitchOutcome(deviceId, { kind: "failed", reason: "switch_unanswered" });
  }, [deviceId, settle]);
  useEffect(() => settle, [settle]);
  const read = useCallback(() => {
    const request = `look-switch-${deviceId}-${Date.now()}`;
    asks.current.look = request;
    setLook({ state: "reading" });
    actions.pairing("look_switch", { device: deviceId, request_id: request });
  }, [actions, deviceId]);
  const press = useCallback(() => {
    const request = `switch-${deviceId}-${Date.now()}`;
    asks.current.press = request;
    setPressed(true);
    useUiStore.getState().setSwitchOutcome(deviceId, null);
    // Asking for the switch is the consent to start a Hide on a machine that runs none.
    actions.pairing("switch", { device: deviceId, install: true, request_id: request });
    if (deadline.current !== null) clearTimeout(deadline.current);
    deadline.current = setTimeout(unanswered, SWITCH_ANSWER_WITHIN_MS);
  }, [actions, deviceId, unanswered]);
  const asked = useRef(false);
  useEffect(() => {
    if (!wanted) {
      asked.current = false;
      asks.current.look = null;
      admin.current = null;
      setLook(null);
      return;
    }
    // A fold keeps what was read; opening the row reads again.
    if (!open && asked.current) return;
    asked.current = true;
    read();
  }, [wanted, open, read]);
  // Subscribed, not selected: two answers in one tick would each replace the store's one slot before a render read it.
  useEffect(
    () =>
      useShellStore.subscribe((state, before) => {
        // A daemon that went away answers nothing it was asked: the press is over, and a read under way was not made.
        if (state.connection !== before.connection && state.connection !== "live") {
          if (asks.current.press !== null) unanswered();
          if (asks.current.look !== null) setLook({ state: "unread", reason: "hide_unavailable" });
          asks.current.look = null;
          return;
        }
        const result = state.pairingResult;
        if (!result || result === before.pairingResult || !result.request_id) return;
        if (result.request_id === asks.current.look) {
          asks.current.look = null;
          setLook(lookOf(result, admin));
          return;
        }
        if (result.request_id !== asks.current.press) return;
        settle();
        if (result.ok && result.switch === "left") setLook(lookOf(result, admin));
        else if (result.ok && result.switch === "held") useUiStore.getState().setSwitchOutcome(deviceId, { kind: "held", by: result.by ?? "", terminals: result.terminals ?? null });
        else if (!result.ok && NOT_REACHED.has(result.reason ?? "")) setLook({ state: "unread", reason: result.reason ?? "" });
        // A switch that is already running goes on drawing its steps.
        else if (!result.ok && result.reason !== "switch_running") useUiStore.getState().setSwitchOutcome(deviceId, { kind: "failed", reason: result.reason ?? "" });
        else if (result.ok && result.switch !== "done") useUiStore.getState().setSwitchOutcome(deviceId, { kind: "failed", reason: result.switch ?? "" });
      }),
    [deviceId, settle, unanswered],
  );
  return { look, pressed, read, press };
}
