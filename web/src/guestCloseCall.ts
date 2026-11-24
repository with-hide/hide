// A window's close of another machine's pane on this computer (PRD
// device-connect-iroh B10, D-74): one `guest_pane_close` per close, answered
// by hided to this window alone with a guest_pane_close_result frame under
// the same request id, so the window moves only once the pane ended and keeps
// its place, with the reason, when it did not.

import type { MessageKey } from "./i18n/catalogs";
import { type CallAnswer, windowCalls } from "./windowCall";

const calls = windowCalls("guest_pane_close", "guest_close_failed");

export const configureGuestCloseCalls = calls.configure;

/** Asks hided to end the pane `paneId` names here. */
export function callGuestClose(paneId: string): Promise<CallAnswer<unknown>> {
  return calls.call({ pane_id: paneId });
}

/** A `guest_pane_close_result` frame. */
export const receiveGuestCloseResult = calls.receive;

/** The socket closed: no answer comes for a close still waiting. */
export const refuseWaitingGuestCloses = calls.refuseWaiting;

/**
 * The one line a close that did not go says, by its code: the pane is no
 * longer listed here, the connection dropped before an answer, this screen
 * may not close it (an older build of that computer), or anything else.
 */
export function guestCloseFailure(code: string): MessageKey {
  if (code === "pane_unavailable") return "pairing.work.closeFailed.gone";
  if (code === "socket_closed") return "pairing.work.closeFailed.offline";
  if (code === "not_here") return "pairing.work.closeFailed.notHere";
  return "pairing.work.closeFailed.other";
}
