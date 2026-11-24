// A call a window waits for, apart from the snapshot: one event of its kind
// per call, answered by one result frame under the same request id, to this
// window alone; a call still waiting when the socket closes is refused
// `socket_closed`, and one the daemon refused for this connection (a
// `refused` frame names the kind, never the request) is refused that frame's
// reason. The automations' form (`automationCall.ts`), Settings › Bots' made
// bots (`accessCall.ts`) and the close of another machine's pane
// (`guestCloseCall.ts`) each keep one.

import type { DispatchFn } from "./ws";

/** The answer, or the code the call was refused with. */
export type CallAnswer<T> = { ok: true; value: T } | { ok: false; code: string };

/** A result frame's payload. */
export type CallResult = { request_id: string; ok: boolean; value?: unknown; code?: string };

/** Each kind's waiting calls, refused together when the daemon refuses that kind on this connection. */
const refusers = new Map<string, (code: string) => void>();

/**
 * The daemon refused `kind` on this connection (`refused` frame): its class
 * does not admit this connection, so every call of it still waiting is
 * refused with `reason`. A kind no call keeps is the store's to note.
 */
export function refuseCallsOf(kind: string, reason: string): void {
  refusers.get(kind)?.(reason);
}

/** The calls of one event `kind`, refused `unavailable` when an answer names no code. */
export function windowCalls(kind: string, unavailable: string) {
  let dispatchFn: DispatchFn | null = null;
  const pending = new Map<string, (answer: CallAnswer<unknown>) => void>();
  let nextRequest = 1;
  const refuseWaiting = (code: string): void => {
    for (const [requestId, resolve] of pending) {
      pending.delete(requestId);
      resolve({ ok: false, code });
    }
  };
  refusers.set(kind, refuseWaiting);
  return {
    /** Sets where calls go and returns where they went before, so a window that answers them itself (the practice window) can put the old one back. */
    configure(dispatch: DispatchFn | null): DispatchFn | null {
      const before = dispatchFn;
      dispatchFn = dispatch;
      return before;
    },
    call<T>(call: object): Promise<CallAnswer<T>> {
      return new Promise((resolve) => {
        const request_id = `${kind}-${nextRequest}`;
        nextRequest += 1;
        if (!dispatchFn) return resolve({ ok: false, code: "socket_closed" });
        pending.set(request_id, resolve as (answer: CallAnswer<unknown>) => void);
        if (dispatchFn({ schema_version: 2, kind, payload: { request_id, call } }) === false) {
          pending.delete(request_id);
          resolve({ ok: false, code: "socket_closed" });
        }
      });
    },
    /** A result frame; one no call waits for (its window reloaded) is dropped. */
    receive(payload: CallResult): void {
      const resolve = pending.get(payload.request_id);
      if (!resolve) return;
      pending.delete(payload.request_id);
      resolve(payload.ok ? { ok: true, value: payload.value } : { ok: false, code: payload.code ?? unavailable });
    },
    /** The socket closed: no answer comes for a call still waiting. */
    refuseWaiting(): void {
      refuseWaiting("socket_closed");
    },
  };
}
