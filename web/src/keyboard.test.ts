import { emptyScope } from "../test/legacyAgentScope";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { commitCycle, panelCycle, projectCycle } from "./keyboard";
import { currentSurface, observeEntries, resetRecent } from "./recent";
import type { SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";

// This Mac holds api (tab l1); the device `mini` holds web (tabs t1, t2), and
// `build-box` is registered but not connected, so it holds nothing current.

function tab(id: string) {
  return { id, label: id, panes: [{ id: `${id}-pane` }], workspace_id: "", checkout_id: "", empty: false, delegated: false };
}

function checkout(id: string, workspace: string, label: string, tabs: string[]) {
  return { agent_scope: emptyScope(), id, workspace_id: workspace, label, path: `/srv/${label}`, tabs: tabs.map((name) => tab(name)), active_tab_id: tabs[0] ?? null, strip: [] };
}

function world(front: "local" | "mini"): SnapshotRest {
  const remoteCheckout = checkout("remote:mini:checkout:c1", "remote:mini:workspace:w1", "web", ["remote:mini:tab:t1", "remote:mini:tab:t2"]);
  return {
    navigator: {
      focused_device_id: front,
      focused_workspace_id: "w-api",
      focused_checkout_id: "c-api",
      devices: [
        { id: "local", label: "This Mac", kind: "local", state: "local", message: null },
        { id: "mini", label: "mini", kind: "remote", state: "ready", message: null },
        { id: "build-box", label: "build-box", kind: "remote", state: "unavailable", message: null },
      ],
      workspaces: [{ agent_scope: emptyScope(), id: "w-api", label: "api", device_id: "local", checkouts: [checkout("c-api", "w-api", "api", ["l1"])] }],
      agents: [],
    },
    status: {
      remote: [
        {
          target_id: "mini",
          state: "connected",
          session: {
            workspaces: [
              { agent_scope: emptyScope(), id: "remote:mini:workspace:w1", label: "web", device_id: "mini", checkouts: [remoteCheckout] },
            ],
            agents: [],
            active_tab_ids: {},
            focused_workspace_id: "remote:mini:workspace:w1",
            focused_checkout_id: remoteCheckout.id,
            focused_tab_id: "remote:mini:tab:t2",
            focused_pane_id: null,
            pane_layouts: [],
          },
        },
        { target_id: "build-box", state: "not_connected", session: null },
      ],
    },
  } as unknown as SnapshotRest;
}

describe("Recent Panels across devices (PRD home-device-rail B39)", () => {
  // This window runs on a Mac, whatever system runs the test (#972 names it by its system).
  beforeEach(() => vi.stubGlobal("navigator", { platform: "MacIntel", userAgent: "" }));
  afterEach(() => vi.unstubAllGlobals());
  beforeEach(() => resetRecent());

  it("lists this Mac's and a connected device's tabs in one order, chipped where the device is not in front", () => {
    const local = world("local");
    observeEntries(local, currentSurface(local, false));
    const mini = world("mini");
    observeEntries(mini, currentSurface(mini, false));
    // From the device in front, this Mac's row carries the chip and the device's own rows do not.
    const fromMini = panelCycle(mini)!;
    expect(fromMini.items.map((item) => [item.title, item.chip?.label ?? null])).toEqual([
      ["remote:mini:tab:t2", null],
      ["l1", "This Mac"],
      ["remote:mini:tab:t1", null],
    ]);
    // From this Mac the device's rows carry it.
    const fromLocal = panelCycle(local)!;
    expect(fromLocal.items.filter((item) => item.chip !== null).map((item) => item.chip?.label)).toEqual(["mini", "mini"]);
    expect(fromLocal.items.some((item) => item.title === "build-box")).toBe(false);
  });

  it("commits a device's tab as one event that also brings the device forward", () => {
    const local = world("local");
    useShellStore.setState({ rest: local });
    observeEntries(local, currentSurface(local, false));
    const sent: { kind: string; payload: Record<string, unknown> }[] = [];
    const actions = createActions((event) => sent.push(event as never));
    const cycle = panelCycle(local)!;
    const index = cycle.items.findIndex((item) => item.title === "remote:mini:tab:t1");
    commitCycle({ ...cycle, index }, actions);
    expect(sent).toHaveLength(1);
    expect(sent[0]).toMatchObject({ kind: "remote_control", payload: { target_id: "mini", action: "focus_tab", tab_id: "remote:mini:tab:t1", focus_device: true } });
  });

  it("lists every connected device's projects for Recent Projects", () => {
    const local = world("local");
    const cycle = projectCycle(local)!;
    expect(cycle.items.map((item) => [item.title, item.chip?.label ?? null])).toEqual([
      ["api", null],
      ["web", "mini"],
    ]);
  });
});
