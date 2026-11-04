import assert from "node:assert/strict";

export type CloseTiming = {
  op_id: string;
  pane_ids: string[];
  outcome: string;
  requested_at_unix_ms: number;
  applied_ms?: number;
};

export type CloseRecord = Partial<CloseTiming> & { kind: string; message?: string; op?: string };

/**
 * These two diagnostics are enqueued under the same Runtime lock and written
 * by diagnostics.rs's single FIFO writer. Their file order witnesses the
 * causal boundary; rounded wall/elapsed times and the delayed timing record
 * do not. A missing or duplicated witness fails, including after log rotation.
 */
export function completedCloseOrder(records: CloseRecord[], timings: CloseTiming[], watched: string[]): string[] {
  assert.deepEqual(timings.flatMap((timing) => timing.pane_ids).sort(), [...watched].sort(), "one timing per watched pane");
  const uniqueIndex = (kind: string, messages: string[]): number => {
    const matches = records.flatMap((record, index) => record.kind === kind && messages.includes(record.message ?? "") ? [index] : []);
    assert.equal(matches.length, 1, `one ${kind} witness for ${messages[0]}`);
    return matches[0]!;
  };
  const closes = timings.map((timing) => {
    assert.equal(timing.pane_ids.length, 1, "one pane per close");
    assert.equal(timing.outcome, "completed", "close completed");
    assert.ok(timing.op_id.startsWith("pane-close-"), "a close operation id");
    const pane = timing.pane_ids[0]!;
    const key = timing.op_id.slice("pane-close-".length);
    assert.ok(key.length > 0, "a close key");
    const requested = uniqueIndex("pane.close.requested", [`Closing pane ${pane}`]);
    const applied = uniqueIndex("recent_closed.topology_confirmed", [
      `Confirmed close ${key} by fresh topology; restore data is reserved`,
      `Confirmed close ${key} by fresh topology; restore data was unavailable`,
    ]);
    assert.ok(requested < applied, `${pane} requested before its topology confirmation`);
    return { pane, requested, applied };
  }).sort((left, right) => left.requested - right.requested);
  for (const [index, close] of closes.entries()) {
    const next = closes[index + 1];
    if (next) assert.ok(close.applied < next.requested, `${close.pane} confirmed before ${next.pane} requested`);
  }
  return closes.map((close) => close.pane);
}
