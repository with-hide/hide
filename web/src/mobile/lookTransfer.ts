// The read of one View in flight on this phone (PRD mobile-companion-ui B23, D-10, D-20): hided sends a head, then chunks
// under a window of four the phone confirms one by one, then an end. One read at a time, named by its request id; a frame of
// any other read is a late frame of one the phone gave up, and is dropped without a confirmation.

import { addChunk, beginAssembly, finishAssembly, type Assembly } from "./lookContent";
import type { ServerFrame } from "./protocol";
import { showLook, usePhone } from "./store";

let assembly: Assembly | null = null;

/** The read in flight is over, finished or given up. */
export function dropTransfer(): void {
  assembly = null;
}

function current(requestId: string): boolean {
  return usePhone.getState().detail?.look?.requestId === requestId;
}

/** Applies a head, a chunk or an end of the read in flight; `confirm` sends the phone's `look_ack` for a chunk it took. */
export function onLookTransfer(frame: ServerFrame, confirm: (requestId: string, seq: number) => void): void {
  switch (frame.type) {
    case "look_head": {
      if (!current(frame.request_id)) return;
      assembly = beginAssembly(frame);
      if (!assembly) showLook(frame.request_id, { state: "failed" });
      return;
    }
    case "look_chunk": {
      if (!assembly || assembly.requestId !== frame.request_id) return;
      const next = addChunk(assembly, frame.seq, frame.data);
      if (!next) {
        assembly = null;
        showLook(frame.request_id, { state: "failed" });
        return;
      }
      assembly = next;
      confirm(frame.request_id, frame.seq);
      return;
    }
    case "look_end": {
      if (!assembly || assembly.requestId !== frame.request_id) return;
      const done = assembly;
      assembly = null;
      showLook(frame.request_id, finishAssembly(done) ?? { state: "failed" });
      return;
    }
    default:
      return;
  }
}
