// Where a key goes right after the operator asked for a new tab or a split
// (PRD instant-pane-topology D-11). From the request until the new pane holds
// the keyboard, keys belong to that pane, which does not exist yet: they are
// sent against the request, and the core delivers them to the pane Herdr's
// answer names, or drops them when the creation fails. The pane that had the
// keyboard never receives them.
//
// The mark ends when the keyboard reaches another pane (the new pane, or one
// the operator chose), when the operator points at a pane or a tab, when the
// core reports the request discarded, or when the core has no row for it: one
// it listed and then dropped, or one it never listed within
// `UNSEEN_REQUEST_LIMIT_MS` (the event never reached it, or the daemon was
// replaced). Once the pane exists, its node owns the hold until its shell
// or forked agent is ready, for every client sending keys to that pane.

import type { InputRequest } from "./snapshot";
import { useShellStore } from "./store";

/** How long a mark waits for the core to list its request at all. */
export const UNSEEN_REQUEST_LIMIT_MS = 5000;

type Armed = {
  requestId: string;
  /**
   * The pane the keyboard comes back to: the terminal holding DOM focus when
   * the request was made, or, from a palette, menu or sidebar, the focused pane.
   */
  originPaneId: string | null;
  armedAt: number;
  /** The core has listed the request. */
  seen: boolean;
  /** The pane the core said this request's keys wait for. */
  heldFor?: string;
};

let armed: Armed | null = null;

/** The pane whose terminal holds DOM focus, if a terminal does. */
function focusedTerminalPane(): string | null {
  if (typeof document === "undefined") return null;
  const active = document.activeElement;
  if (!(active instanceof HTMLElement)) return null;
  return active.closest<HTMLElement>("[data-terminal-host]")?.dataset.terminalHost ?? null;
}

/** Marks every key from now on as the creation `requestId`'s, until the mark ends. */
export function armKeyTarget(requestId: string, now: number = Date.now()) {
  armed = { requestId, originPaneId: focusedTerminalPane() ?? useShellStore.getState().focusedPaneId ?? null, armedAt: now, seen: false };
}

export function disarmKeyTarget() {
  armed = null;
}

/** The request keys go to while one is armed (test seam). */
export function armedKeyRequest(): string | null {
  return armed?.requestId ?? null;
}

/** The `key` payload for bytes typed in `paneId`'s terminal. */
export function keyPayload(paneId: string, bytesBase64: string, now: number = Date.now()): { pane_id: string; bytes_base64: string } | { pending_request: string; bytes_base64: string } {
  if (armed && !armed.seen && now - armed.armedAt > UNSEEN_REQUEST_LIMIT_MS) armed = null;
  if (armed) return { pending_request: armed.requestId, bytes_base64: bytesBase64 };
  return { pane_id: paneId, bytes_base64: bytesBase64 };
}

/**
 * A terminal took DOM focus. Focus that lands on a pane other than the one the
 * request was made from means the keyboard has moved on, so keys follow it.
 * Focus coming back to the origin pane keeps the mark: the new pane is still
 * where the operator's keys belong.
 */
export function noteTerminalFocus(paneId: string) {
  if (!armed || paneId === armed.originPaneId) return;
  armed = null;
}

/**
 * The operator pointed at a pane, a tab or an area: they chose where they are.
 * The node holds keys for a pane whose creation is still starting.
 */
export function noteOperatorPointer() {
  armed = null;
}

/** Reads the core's input request rows; a request it discarded ends the mark. */
export function observeInputRequests(requests: InputRequest[] | undefined) {
  if (!armed) return;
  const row = requests?.find((request) => request.request_id === armed?.requestId);
  if (!row) {
    if (armed.seen) armed = null;
    return;
  }
  if (row.state === "discarded") armed = null;
  // Once the pane has the keyboard, bare pane keys go straight to its node.
  // That node owns the fork hold for every client. This mark only carries
  // keys typed before focus arrives from the source to the creation request.
  else if (row.state === "ready" && armed.heldFor && focusedTerminalPane() === armed.heldFor) armed = null;
  else {
    armed.seen = true;
    if (row.state === "pending" && row.held_for) armed.heldFor = row.held_for;
  }
}
