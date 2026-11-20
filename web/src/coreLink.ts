// A node's link to its core as its window sees it: the `core_link` frame the
// node's daemon sends while its screens wait for the core on another machine
// (docs/ARCHITECTURE.md, A core on another machine). Nothing here decides the
// link; the window asks with one `core_link` event and draws what the frames
// say.

import type { TFunction } from "i18next";
import type { DeviceProblem, DeviceSignIn } from "./snapshot";

/**
 * Where a node's link to its core stands while its screens wait (`core_link` frame); `disconnected` is the operator's
 * own end of it (B16), and `stopped` a failure only the operator can fix, which nothing dials again until they retry;
 * `problem` is that ssh failure as the device row groups it, null in any other phase. `machine` is
 * the core machine's name once its hello gave it; `alias` is the SSH alias the node dials it by, the name a window
 * that has not drawn the core yet has. `sign_in` is what ssh asks the dial the operator's connect or Retry started
 * (PRD system-ssh-device-link B33), which the strip shows as the device row does.
 */
export type CoreLink = {
  phase: "connecting" | "updating" | "waiting" | "stopped" | "disconnected";
  machine: string | null;
  alias: string;
  problem?: DeviceProblem | null;
  sign_in?: DeviceSignIn | null;
  /** Whether this machine becomes a Hide of its own once it reaches its core (PRD device-connect-iroh B45): only in a build that offers connections between two Hides. */
  splits?: boolean;
};

/**
 * Why this machine was not made a Hide of its own, paired with its core's machine, on this connection (PRD
 * device-connect-iroh B45): the core's machine was not reached over SSH, its Hide refused, the two did not meet on the
 * connection two Hides use, or this machine could not save its side. The node goes on as before, and the window's own
 * device row says the cause with a Retry.
 */
export type SplitFailure = "split_not_reached" | "split_refused" | "split_not_connected" | "split_not_saved";

/**
 * What a node's daemon tells a screen the core draws (`core_link` frame, phase `live`): only what the core cannot
 * know of this machine's link.
 */
export type CoreLinkLive = { phase: "live"; split_failed: SplitFailure | null };

/** The one line the window's own device row says for a split that did not happen; a cause this build does not name reads as a refusal. */
export function splitFailureLine(failure: SplitFailure, machine: string, t: TFunction<"translation">): string {
  switch (failure) {
    case "split_not_reached":
      return t("coreLink.split.notReached", { machine });
    case "split_not_connected":
      return t("coreLink.split.notConnected", { machine });
    case "split_not_saved":
      return t("coreLink.split.notSaved", { machine });
    default:
      return t("coreLink.split.refused", { machine });
  }
}
