import { describe, expect, it } from "vitest";
import { failedReceiptMoved, inlineStartAnswer, receiptTarget } from "./inlineStart";
import type { AgentStartReceipt } from "./agentLayout";
import type { SnapshotRest } from "./snapshot";
import { emptyScope, legacyRest } from "../test/legacyAgentScope";
const receipt: AgentStartReceipt = { request_id: "start-original", group_id: "g1", area_id: "a1", draft_tab_id: "draft-1", live_tab_id: "live-1", pane_id: "pane-1", add_pane: false, state: "ready", agent_kind: null, agent_phase: null };
describe("durable inline start answers", () => {
  it("keeps a delegated canvas's failed add intent while its original group and area still exist", () => {
    const failed: AgentStartReceipt = { ...receipt, state: "failed", draft_tab_id: null, live_tab_id: "child", add_pane: true };
    const rest = legacyRest({ navigator: { workspaces: [{ id: "project", path: "/fixture", device_id: "local", label: "Fixture", registered: true, temporary: false, pinned: false, agent_scope: emptyScope(), inactive_checkouts: { expanded: false, checkout_ids: [] }, checkouts: [{ id: "checkout", workspace_id: "project", label: "Fixture", path: "/fixture", branch: null, purpose: null, is_worktree: false,  exists: true, has_panes: true, pull_request: null, active_tab_id: "child", strip: [], next_tab_label: "1", tabs: [{ id: "child", workspace_id: "project", checkout_id: "checkout", label: "Child", empty: false, delegated: true, panes: [] }], agent_scope: emptyScope(), agent_groups: { selected: "g1", groups: [{ id: "g1", name: "Work", tabs: ["parent"] }], drafts: [], start_receipts: [failed] } }] }] }, workspace_view: { device_id: "local", path: "/fixture", open: false, documents: false, tool: null, pinned: false, document_width: null, detail_width: null, explorer_width: null, panel_width: null, lane_called: 0, lane_calls: 0, lane_called_side: "view", agent_layout: { root: { area: { id: "a1", active: "parent", displays: [{ id: "parent" }] } }, active_area: "a1", limits: { areas: 6, depth: 3, displays: 64 }, display_count: 1, canvases: { a1: "child" }, group_id: "g1" } } }, []);
    expect(failedReceiptMoved(rest, "local", "/fixture", failed)).toBe(false);
    const layout = rest.workspace_view!.agent_layout!;
    layout.canvases = {};
    // Navigating away does not move the delegated tab or invalidate its intent.
    expect(failedReceiptMoved(rest, "local", "/fixture", failed)).toBe(false);
    layout.root = { area: { id: "a2", active: "parent", displays: [{ id: "parent" }] } };
    expect(failedReceiptMoved(rest, "local", "/fixture", failed)).toBe(true);
    layout.root = { area: { id: "a1", active: "parent", displays: [{ id: "parent" }] } };
    rest.navigator!.workspaces![0]!.checkouts[0]!.agent_groups!.groups = [{ id: null, name: null, tabs: ["parent"] }];
    expect(failedReceiptMoved(rest, "local", "/fixture", failed)).toBe(true);
  });
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
