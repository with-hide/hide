import { expect, test } from "vitest";
import { completedCloseOrder, type CloseRecord, type CloseTiming } from "./close-order";

// Reviewer counterexample: request at 1000.1, confirmation at 1002.8,
// next request at 1002.9. The emitted fields are rounded independently.
const timings: CloseTiming[] = [
  { op_id: "pane-close-child", pane_ids: ["w1:p3"], outcome: "completed", requested_at_unix_ms: 1000, applied_ms: 2.7 },
  { op_id: "pane-close-parent", pane_ids: ["w1:p2"], outcome: "completed", requested_at_unix_ms: 1002, applied_ms: 0.1 },
];
const requested = (pane: string): CloseRecord => ({ kind: "pane.close.requested", message: `Closing pane ${pane}` });
const confirmed = (key: string): CloseRecord => ({ kind: "recent_closed.topology_confirmed", message: `Confirmed close ${key} by fresh topology; restore data is reserved` });
const watched = ["w1:p3", "w1:p2"];

test("a child confirmed before its parent's request passes despite rounded times and delayed timing lines", () => {
  const records = [requested("w1:p3"), confirmed("child"), requested("w1:p2"), confirmed("parent")];
  expect(completedCloseOrder(records, [...timings].reverse(), watched)).toEqual(watched);
  const sameMillisecond = timings.map((timing) => ({ ...timing, requested_at_unix_ms: 1000, applied_ms: 0 }));
  expect(completedCloseOrder(records, sameMillisecond, watched)).toEqual(watched);
});

test("a parent requested before child confirmation fails even when both times round to the same millisecond", () => {
  const records = [requested("w1:p3"), requested("w1:p2"), confirmed("child"), confirmed("parent")];
  const sameMillisecond = timings.map((timing) => ({ ...timing, requested_at_unix_ms: 1000, applied_ms: 0 }));
  expect(() => completedCloseOrder(records, sameMillisecond, watched)).toThrow("w1:p3 confirmed before w1:p2 requested");
});

test("an old or unrelated confirmation cannot replace this close's missing witness", () => {
  const records = [confirmed("old-child"), requested("w1:p3"), requested("w1:p2"), confirmed("parent")];
  expect(() => completedCloseOrder(records, timings, watched)).toThrow("one recent_closed.topology_confirmed witness");
});

test("duplicate requests or a missing pane fail instead of inventing an order", () => {
  const records = [requested("w1:p3"), requested("w1:p3"), confirmed("child"), requested("w1:p2"), confirmed("parent")];
  expect(() => completedCloseOrder(records, timings, watched)).toThrow("one pane.close.requested witness");
  expect(() => completedCloseOrder(records, [timings[0]!], watched)).toThrow("one timing per watched pane");
});
