import { describe, expect, it } from "vitest";
import { movingTab, movePaneReason, paneEdge, selectedMovePane } from "./paneMove";
import type { AsyncOperation, PaneRow, Tab, TerminalPane } from "./snapshot";

const pane: PaneRow = { id: "w1:p1", herdr_label: "shell", terminal_title: null, cwd: "/fixture", status_code: "idle", requires_close_confirmation: false, requires_close_status_check: false, identity_label: null };
const operation = (phase: string): AsyncOperation => ({ id: "request", kind: "pane.move", target_id: pane.id, scope_id: "source", related_scope_ids: ["source", "target"], phase, stage: "move", message: null, retryable: phase === "failed" });
const transport = (state: string): TerminalPane => ({ pane_id: pane.id, closed: false, exit_code: null, transport_state: state, transport_message: null });

describe("pane move admission", () => {
  it("reserves exactly the related tabs until definite completion or refusal", () => {
    for (const phase of ["preparing", "transmitting", "awaiting_topology", "unknown"]) {
      expect(movingTab([operation(phase)], "source")).toBe(true);
      expect(movingTab([operation(phase)], "target")).toBe(true);
      expect(movingTab([operation(phase)], "other")).toBe(false);
    }
    for (const phase of ["failed", "completed"]) expect(movingTab([operation(phase)], "source")).toBe(false);
  });
  it("allows an unattached background pane while refusing sleep, startup and closing", () => {
    expect(movePaneReason(pane, "source", [], [])).toBeNull();
    expect(movePaneReason(pane, "source", [transport("released")], [])).toBeNull();
    expect(movePaneReason(pane, "source", [transport("starting")], [])).toBe("starting");
    expect(movePaneReason(pane, "source", [transport("closing")], [])).toBe("pending");
    expect(movePaneReason(pane, "source", [], [operation("unknown")])).toBe("pending");
    expect(movePaneReason({ ...pane, failed_terminal_id: "failed" }, "source", [], [])).toBe("unavailable");
  });
  it("never guesses a selected target when several panes have no confirmed selection", () => {
    const tab: Tab = { id: "target", workspace_id: "w1", checkout_id: "checkout", label: null, empty: false, delegated: false, panes: [pane, { ...pane, id: "w1:p2" }] };
    expect(selectedMovePane(tab)).toBeNull();
    expect(selectedMovePane({ ...tab, sidebar: { manual_name: null, number: 1, named_by_operator: false, focused_pane_id: "w1:p2" } })).toBe("w1:p2");
    expect(selectedMovePane({ ...tab, panes: [pane] })).toBe(pane.id);
  });
});

describe("edge destinations", () => {
  const rect = { left: 100, top: 50, width: 400, height: 200 };
  it.each([[110, 150, "left"], [490, 150, "right"], [300, 60, "up"], [300, 240, "down"]] as const)("resolves %s,%s to %s", (x, y, edge) => expect(paneEdge(rect, x, y)).toBe(edge));
  it("rejects the center, outside and empty geometry", () => {
    expect(paneEdge(rect, 300, 150)).toBeNull();
    expect(paneEdge(rect, 90, 150)).toBeNull();
    expect(paneEdge({ ...rect, width: 0 }, 100, 50)).toBeNull();
  });
});
