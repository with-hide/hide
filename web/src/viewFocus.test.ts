import { describe, expect, it } from "vitest";
import { closeShortcutPolicy, newTabPolicy, type KeyboardOwner } from "./viewFocus";

describe("close shortcut policy", () => {
  const decide = (owner: KeyboardOwner, displayId: string | null = "d1", paneIds = ["p1", "p2"], detailShown = false) =>
    closeShortcutPolicy({ owner, workspace: "workspace", displayId, paneIds, detailShown });

  it("closes the view even while an agent pane remains focused in the core", () => {
    expect(decide({ kind: "view", workspace: "workspace", areaId: "a1" })).toEqual({ kind: "view", id: "d1" });
  });

  it("closes only the keyboard's pane in a split tab or a one-pane tab", () => {
    const owner: KeyboardOwner = { kind: "pane", workspace: "workspace", paneId: "p2" };
    expect(decide(owner)).toEqual({ kind: "pane", id: "p2" });
    expect(decide(owner, null, ["p2"])).toEqual({ kind: "pane", id: "p2" });
  });

  it("never falls through from tools, no focus, a retired view or a retired pane", () => {
    expect(decide({ kind: "tool", workspace: "workspace" })).toEqual({ kind: "nothing", reason: "the lane's tool owns the keyboard" });
    expect(decide({ kind: "agent", workspace: "workspace" })).toMatchObject({ kind: "nothing" });
    expect(decide({ kind: "none" })).toEqual({ kind: "nothing", reason: "no keyboard owner" });
    expect(decide({ kind: "view", workspace: "workspace", areaId: "gone" }, null)).toMatchObject({ kind: "nothing" });
    expect(decide({ kind: "pane", workspace: "workspace", paneId: "gone" })).toMatchObject({ kind: "nothing" });
    expect(decide({ kind: "pane", workspace: "elsewhere", paneId: "p1" })).toMatchObject({ kind: "nothing" });
  });

  it("closes the 보기 slot from the slot or from nowhere, leaves the tool column closing nothing, and a pane or a View its own (PRD lane-two-columns B13, B16)", () => {
    const shown = (owner: KeyboardOwner) => decide(owner, "d1", ["p1"], true);
    expect(shown({ kind: "detail", workspace: "workspace" })).toEqual({ kind: "detail" });
    expect(shown({ kind: "tool", workspace: "workspace" })).toMatchObject({ kind: "nothing" });
    expect(shown({ kind: "none" })).toEqual({ kind: "detail" });
    expect(shown({ kind: "pane", workspace: "workspace", paneId: "p1" })).toEqual({ kind: "pane", id: "p1" });
    expect(shown({ kind: "view", workspace: "workspace", areaId: "a1" })).toEqual({ kind: "view", id: "d1" });
    expect(shown({ kind: "agent", workspace: "workspace" })).toMatchObject({ kind: "nothing" });
    expect(shown({ kind: "detail", workspace: "elsewhere" })).toMatchObject({ kind: "nothing" });
  });
});

describe("new tab policy", () => {
  const decide = (owner: KeyboardOwner, viewAreaIds = ["v1", "v2"], workspace: string | null = "workspace") =>
    newTabPolicy({ owner, workspace, viewAreaIds, paneAreas: { p1: "a1", p2: "a2" } });

  it("opens in the View area that holds the keyboard, not the View active area", () => {
    expect(decide({ kind: "view", workspace: "workspace", areaId: "v2" })).toEqual({ kind: "view", areaId: "v2" });
  });

  it("opens in the Agent area showing the keyboard's pane", () => {
    expect(decide({ kind: "pane", workspace: "workspace", paneId: "p2" })).toEqual({ kind: "agent", areaId: "a2" });
  });

  it("keeps the Agent active area for anything else", () => {
    const active = { kind: "agent", areaId: null };
    expect(decide({ kind: "none" })).toEqual(active);
    expect(decide({ kind: "tool", workspace: "workspace" })).toEqual(active);
    expect(decide({ kind: "agent", workspace: "workspace" })).toEqual(active);
    expect(decide({ kind: "view", workspace: "workspace", areaId: "v1" }, [])).toEqual(active);
    expect(decide({ kind: "view", workspace: "workspace", areaId: "v1" }, ["v1"], null)).toEqual(active);
    expect(decide({ kind: "pane", workspace: "elsewhere", paneId: "p1" })).toEqual(active);
    expect(decide({ kind: "pane", workspace: "workspace", paneId: "gone" })).toEqual(active);
  });
});
