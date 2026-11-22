// Settings › Bots' call on the bots the operator makes (PRD hide-external-
// control, operator-made bots): make one, give it a new code, or read its live
// code again for the sentence the window copies. The answer is the only place a
// code reaches the screen: hided sends it to this machine's own window alone
// (`hided/src/access_screen.rs`), never in the snapshot, whose `invites` carry
// the name, mode and expiry.

import type { AccessMode } from "./snapshot";
import { type CallAnswer, windowCalls } from "./windowCall";

/** The calls Settings › Bots makes (`access_call`). */
export type AccessCall = { call: "invite"; name: string; mode: AccessMode } | { call: "renew"; id: string } | { call: "code"; id: string };

/** A made bot with its code, shown in groups of four (`K7Q4-MZ2P-X3RD`). */
export type MadeBot = { id: string; name: string; mode: AccessMode; code: string; expires_ms: number };

const calls = windowCalls("access_call", "access_unavailable");

export const configureAccessCalls = calls.configure;

export function callAccess(call: AccessCall): Promise<CallAnswer<MadeBot>> {
  return calls.call<MadeBot>(call);
}

/** An `access_result` frame. */
export const receiveAccessResult = calls.receive;

/** The socket closed: no answer comes for a call still waiting. */
export const refuseWaitingAccessCalls = calls.refuseWaiting;
