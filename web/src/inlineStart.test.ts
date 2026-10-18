import { describe, expect, it } from "vitest";
import { inlineStartAnswer, receiptTarget } from "./inlineStart";
import type { AgentStartReceipt } from "./agentLayout";
import type { SnapshotRest } from "./snapshot";
const receipt: AgentStartReceipt = { request_id: "start-original", group_id: "g1", area_id: "a1", draft_tab_id: "draft-1", live_tab_id: "live-1", pane_id: "pane-1", add_pane: false, state: "ready", agent_kind: null, agent_phase: null };
describe("durable inline start answers", () => {
  it("accepts late success over an old error without inventing a new target", () => {
    const rest = { status: { last_error: { request_id: receipt.request_id, message: "Timeout" } } } as SnapshotRest;
    expect(inlineStartAnswer(rest, receipt.request_id, receipt)).toMatchObject({ phase: "ready", paneId: "pane-1" });
    expect(receiptTarget(receipt)).toEqual({ group_id: "g1", area_id: "a1", tab_id: "draft-1", add_pane: false });
  });
  it("requires original-intent confirmation after restart, including an unknown launched agent", () => {
    expect(inlineStartAnswer(null, receipt.request_id, { ...receipt, state: "pending" })).toEqual({ phase: "pending" });
    expect(inlineStartAnswer(null, receipt.request_id, { ...receipt, agent_kind: "claude", agent_phase: "unknown" })).toEqual({ phase: "pending" });
    expect(receiptTarget({ ...receipt, draft_tab_id: null, add_pane: true })).toMatchObject({ tab_id: "live-1", add_pane: true });
  });
});
