import type { TFunction } from "i18next";
import type { CoreLink } from "./coreLink";
import { deviceProblemLine, retryCanHelp, retrySignsIn } from "./settings";
import type { DeviceSignIn } from "./snapshot";

export type ConnectionState = "connecting" | "live" | "reconnecting" | "gone";

export const BACKOFF_START_MS = 500;
export const BACKOFF_MAX_MS = 30_000;
export const HEALTH_FAILS_TO_GONE = 3;

export function nextBackoff(currentMs: number): number {
  return Math.min(BACKOFF_MAX_MS, Math.max(BACKOFF_START_MS, currentMs * 2));
}

export function connectionAfterHealthFails(fails: number): ConnectionState {
  return fails >= HEALTH_FAILS_TO_GONE ? "gone" : "reconnecting";
}

export function badgeText(state: ConnectionState, refused: boolean, t: TFunction<"translation">): string {
  switch (state) {
    case "connecting":
      return t("shell.connection.connecting");
    case "live":
      return "";
    case "reconnecting":
      return t("shell.connection.reconnecting");
    case "gone":
      return refused ? t("shell.connection.refused") : t("shell.connection.gone");
  }
}

/**
 * The strip above the window, in the place and size the connection badge always had (D-08); `reconnect` offers the
 * operator's way back after they ended the link (B16), `retry` the dial a link that stopped on what only they can fix
 * waits for, and `sign_in` the same dial where the link stopped on a sign-in. A link that stopped
 * on a changed server key carries the terminal `command` that checks it and no retry, as the device row does; `signIn` is what ssh
 * asks the dial the operator started, which the strip shows below its line (PRD system-ssh-device-link B33).
 */
export type WindowStrip = {
  kind: ConnectionState | "updating" | "unreachable" | "stopped" | "disconnected" | "sign_in";
  text: string;
  mark: "pending" | "warn" | null;
  action?: "reconnect" | "retry" | "sign_in";
  command?: string;
  signIn?: DeviceSignIn;
};

/**
 * What the strip says while the link waits for the operator, by the ssh failure it stopped on as the device row
 * groups it: a sign-in their Sign in asks for, a changed server key they check in a terminal, a Hide an update replaced
 * that only opening Hide again mends, or a connection to retry.
 */
function stoppedText(link: CoreLink, machine: string, t: TFunction<"translation">): string {
  switch (link.problem) {
    case "sign_in_needed":
      return t("coreLink.strip.signIn", { machine });
    case "sign_in_refused":
      return t("coreLink.strip.signInRefused", { machine });
    case "host_key_changed":
      return t("coreLink.strip.hostKeyChanged", { machine });
    case "hide_updated":
      return t("coreLink.strip.hideUpdated", { machine });
    default:
      return t("coreLink.strip.stopped", { machine, alias: link.alias });
  }
}

/**
 * What the strip says (PRD core-host-node-move B16): the node's update of its
 * core, its failed link or the link the operator ended while its screen waits
 * for the core, else the connection badge.
 * `coreMachine` is the core's machine as the window last knew it, for a
 * node whose link has not named it yet.
 */
export function windowStrip(
  input: { connection: ConnectionState; refused: boolean; link: CoreLink | null; coreMachine: string },
  t: TFunction<"translation">,
): WindowStrip | null {
  const { connection, link } = input;
  if (connection !== "live" && link) {
    // The core's machine as the window's rows name it once it drew the core; before that by the name the
    // core's hello gave, which is its host name, or the alias it is dialed by.
    const machine = input.coreMachine || link.machine || link.alias;
    if (link.sign_in) return { kind: "sign_in", text: t("coreLink.strip.signingIn", { machine }), mark: "pending", signIn: link.sign_in };
    switch (link.phase) {
      case "updating":
        return { kind: "updating", text: t("coreLink.strip.updating", { machine }), mark: "pending" };
      case "disconnected":
        return { kind: "disconnected", text: t("coreLink.strip.disconnected", { machine }), mark: "warn", action: "reconnect" };
      case "waiting":
        // A machine that is split from its core once it reaches it waits for that; any other only reconnects.
        return { kind: "unreachable", text: t(link.splits ? "coreLink.strip.splitWaits" : "coreLink.strip.unreachable", { machine }), mark: "warn" };
      case "stopped": {
        // A changed server key is checked in a terminal and a Hide an update replaced is opened again; neither offers
        // a Retry, as on the device row: a retry would meet the same key, or start the same replaced program.
        const command = deviceProblemLine(link.problem, link.alias, t)?.command ?? undefined;
        if (!retryCanHelp(link.problem)) return { kind: "stopped", text: stoppedText(link, machine, t), mark: "warn", ...(command ? { command } : {}) };
        return { kind: "stopped", text: stoppedText(link, machine, t), mark: "warn", action: retrySignsIn(link.problem) ? "sign_in" : "retry" };
      }
      case "connecting":
        break;
    }
  }
  const text = badgeText(connection, input.refused, t);
  return text ? { kind: connection, text, mark: null } : null;
}
