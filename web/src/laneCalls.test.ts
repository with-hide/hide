import { afterEach, beforeEach, describe, expect, it } from "vitest";
import "./laneCalls";
import type { SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { NO_CALLS_SEEN, type LaneSide, type Tool } from "./workspace";

// A call from outside the page draws the column the core says it brought
// forward in a stacked lane (PRD lane-two-columns B19): a reveal into the
// Explorer the tool column already shows is the tool column's, and an
// open the viewing column's, whether or not the tool changed.

const view = (lane_called: number, lane_calls: number, lane_called_side: LaneSide, tool: Tool | null) => ({
  device_id: "local", path: "/p", open: true, documents: true, tool, pinned: false, 
  document_width: null, detail_width: null, explorer_width: null, panel_width: null, lane_called, lane_calls, lane_called_side,
});

describe("lane calls", () => {
  let saved: ReturnType<typeof useShellStore.getState>;
  let savedUi: ReturnType<typeof useUiStore.getState>;
  beforeEach(() => { saved = useShellStore.getState(); savedUi = useUiStore.getState(); });
  afterEach(() => { useShellStore.setState(saved, true); useUiStore.setState(savedUi, true); });

  const publish = (next: ReturnType<typeof view>) => useShellStore.setState({ rest: { workspace_view: next } as unknown as SnapshotRest });

  it("draws the column the core recorded for the call, not one guessed from the tool", () => {
    useUiStore.setState({ laneCallsSeen: NO_CALLS_SEEN, stackFront: "view" });
    publish(view(1, 1, "view", "explorer"));
    expect(useUiStore.getState().stackFront).toBe("view");
    // A reveal into the Explorer already shown: the tool did not change.
    publish(view(2, 2, "tool", "explorer"));
    expect(useUiStore.getState().stackFront).toBe("tool");
    // An open with the same tool: the documents.
    publish(view(3, 3, "view", "explorer"));
    expect(useUiStore.getState().stackFront).toBe("view");
    // A snapshot that numbers no new call moves nothing.
    useUiStore.setState({ stackFront: "tool" });
    publish(view(3, 3, "view", "changes"));
    expect(useUiStore.getState().stackFront).toBe("tool");
  });
});
