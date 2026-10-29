import type { TFunction } from "i18next";
import { giveUpOffered, movingStripText, type CoreLink, type MoveMachines, type MoveView } from "./coreMove";

/** `moving`: a move holds the window until the machine that takes the core draws it; input is refused meanwhile. */
export type ConnectionState = "connecting" | "live" | "reconnecting" | "gone" | "moving";

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
    case "moving":
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
 * waits for, and `give_up` the move's wait on its other machine where the
 * move offers it (amendment 4), which only opens the move dialog at its confirm.
 */
export type WindowStrip = {
  kind: ConnectionState | "updating" | "unreachable" | "stopped" | "disconnected";
  text: string;
  mark: "pending" | "warn" | null;
  action?: "reconnect" | "retry" | "give_up";
};

/**
 * What the strip says while the link waits for the operator: what they can do in a terminal before they retry, by
 * the ssh failure it stopped on.
 */
function stoppedText(link: CoreLink, machine: string, t: TFunction<"translation">): string {
  switch (link.problem) {
    case "sign_in_needed":
      return t("coreMove.strip.signIn", { machine, alias: link.alias });
    case "host_key_changed":
      return t("coreMove.strip.hostKeyChanged", { machine });
    default:
      return t("coreMove.strip.stopped", { machine, alias: link.alias });
  }
}

/**
 * What the strip says (PRD core-host-node-move W1 to W3, B16): the move's
 * step while it holds the window, the node's update of its core, its failed
 * link or the link the operator ended while its screen waits for the core,
 * else the connection badge.
 * `coreMachine` is the core's machine as the window last knew it, for a
 * node whose link has not named it yet.
 */
export function windowStrip(
  input: { connection: ConnectionState; refused: boolean; move: MoveView | null; link: CoreLink | null; machines: MoveMachines | null; coreMachine: string },
  t: TFunction<"translation">,
): WindowStrip | null {
  const { connection, move, link } = input;
  if (connection === "moving" && move && input.machines) {
    const strip: WindowStrip = { kind: "moving", text: movingStripText(move, input.machines, t), mark: "pending" };
    return giveUpOffered(move) ? { ...strip, action: "give_up" } : strip;
  }
  if (connection !== "live" && link && link.phase !== "connecting") {
    // The core's machine as the window's rows name it once it drew the core; before that by the name the
    // core's hello gave, which is its host name, or the alias it is dialed by.
    const machine = input.coreMachine || link.machine || link.alias;
    switch (link.phase) {
      case "updating":
        return { kind: "updating", text: t("coreMove.strip.updating", { machine }), mark: "pending" };
      case "disconnected":
        return { kind: "disconnected", text: t("coreMove.strip.disconnected", { machine }), mark: "warn", action: "reconnect" };
      case "waiting":
        return { kind: "unreachable", text: t("coreMove.strip.unreachable", { machine }), mark: "warn" };
      case "stopped":
        return { kind: "stopped", text: stoppedText(link, machine, t), mark: "warn", action: "retry" };
    }
  }
  const text = badgeText(connection, input.refused, t);
  return text ? { kind: connection, text, mark: null } : null;
}
