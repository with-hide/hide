import { afterEach, beforeEach, describe, expect, it } from "vitest";
import "./laneCalls";
import type { SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { NO_CALLS_SEEN, NO_DRAWN_PAGES, type LaneSide, type Tool } from "./workspace";

// A call from outside the page draws the column the core says it brought
// forward in a stacked lane (PRD lane-two-columns B19): a reveal into the
// Explorer the tool column already shows is the tool column's, and an
// open the viewing column's, whether or not the tool changed.

const view = (lane_called: number, lane_calls: number, lane_called_side: LaneSide, tool: Tool | null) => ({
  device_id: "local", path: "/p", open: true, tray: { tab_id: "t1", shown: true, count: 0 }, tool, document_width: null, explorer_width: null, panel_width: null, lane_called, lane_calls, lane_called_side,
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

  // A call widens the lane for a page its Workspace did not draw last (PRD
  // browser-width-zoom D-04; lane-desk-pair-view B2): ⌘E and the desk button
  // over the same page keep the width the operator dragged.
  describe("a page called in front", () => {
    const page = (id: string) => ({ id, kind: "browser", url: "http://localhost:5173/", load: 1 });
    let calls = 0;
    const show = (front: string, displays: string[], lane: { open?: boolean; shown?: boolean; call?: boolean } = {}) => {
      if (lane.call !== false) calls += 1;
      publish({
        ...view(calls, calls, "view", null),
        open: lane.open ?? true,
        tray: { tab_id: "t1", shown: lane.shown ?? true, count: displays.length },
        layout: { root: { area: { id: "area-1", active: front, displays: displays.map(page) } }, active_area: "area-1", display_count: displays.length },
      } as ReturnType<typeof view>);
    };
    const widened = () => {
      const asked = useUiStore.getState().pageWiden !== null;
      useUiStore.setState({ pageWiden: null });
      return asked;
    };
    beforeEach(() => {
      calls = 0;
      useUiStore.setState({ laneCallsSeen: NO_CALLS_SEEN, drawnPages: NO_DRAWN_PAGES, pageWiden: null });
      show("p1", ["p1"], { call: false });
    });

    it("does not widen when the lane closes and opens again over the same page", () => {
      show("p2", ["p1", "p2"]);
      expect(widened()).toBe(true);
      show("p2", ["p1", "p2"], { open: false, call: false });
      show("p2", ["p1", "p2"]);
      expect(widened()).toBe(false);
    });

    it("does not widen when the desk folds and unfolds over the same page", () => {
      show("p2", ["p1", "p2"]);
      expect(widened()).toBe(true);
      show("p2", ["p1", "p2"], { shown: false, call: false });
      show("p2", ["p1", "p2"]);
      expect(widened()).toBe(false);
    });

    it("widens for a page that came while the lane was closed when the desk button brings it up", () => {
      show("p1", ["p1"], { open: false, call: false });
      show("p2", ["p1", "p2"], { open: false, call: false });
      show("p2", ["p1", "p2"]);
      expect(widened()).toBe(true);
    });
  });
});
