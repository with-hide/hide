// A node's link to its core as its window sees it: the `core_link` frame the
// node's daemon sends while its screens wait for the core on another machine
// (docs/ARCHITECTURE.md, A core on another machine). Nothing here decides the
// link; the window asks with one `core_link` event and draws what the frames
// say.

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
};
