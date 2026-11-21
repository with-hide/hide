import { describe, expect, it } from "vitest";
import { guestGroups, guestPaneRefuses, guestPanes, guestTabs, isGuestPaneId, peerRows } from "./guestWork";
import type { SnapshotRest, Workspace } from "./snapshot";

const KEY = "cd".repeat(32);

function project(id: string, path: string): Workspace {
  return { id, label: id, path, device_id: "local", checkouts: [{ id: `${id}-main`, path }] } as unknown as Workspace;
}

function rest(panes: { pane: string; folder?: string; since?: number }[]): SnapshotRest {
  return {
    navigator: {
      peers: [{ key: KEY, device_id: "macbook", name: "MacBook Air", connected: false }],
      guest_panes: [{ device_id: "macbook", key: KEY, panes: panes.map((pane) => ({ id: `guest:macbook:pane:${pane.pane}`, pane: pane.pane, since_unix_ms: pane.since ?? 1, folder: pane.folder })) }],
    },
  } as SnapshotRest;
}

describe("another machine's work on this computer (PRD device-connect-iroh B10, B68)", () => {
  it("names a pane by its folder and its machine's letters, whether or not that machine is connected", () => {
    const [pane] = guestPanes(rest([{ pane: "w1:p1", folder: "/work/shop-api" }]), []);
    expect(pane).toMatchObject({ id: "guest:macbook:pane:w1:p1", machine: "MacBook Air", mark: "MA", label: "shop-api", folder: "/work/shop-api", projectId: null });
    // A pane whose node said no folder goes by its own id there.
    expect(guestPanes(rest([{ pane: "w1:p2" }]), [])[0]).toMatchObject({ label: "w1:p2", folder: null });
  });

  it("stands a pane under this computer's project that holds its folder, and under its machine's name otherwise", () => {
    const projects = [project("shop", "/work/shop-api"), project("shop-web", "/work/shop-api/web"), project("other", "/work/shop")];
    const panes = guestPanes(rest([{ pane: "w1:p1", folder: "/work/shop-api/web/src" }, { pane: "w1:p2", folder: "/work/shop-api" }, { pane: "w1:p3", folder: "/scratch/try", since: 0 }, { pane: "w1:p4", folder: "/work/shop-api-2" }]), projects);
    const placed = Object.fromEntries(panes.map((pane) => [pane.label, pane.projectId]));
    // The longest checkout that holds the folder is its project; a folder beside one is in none.
    expect(placed).toEqual({ src: "shop-web", "shop-api": "shop", try: null, "shop-api-2": null });
    expect(guestGroups(panes)).toEqual([{ deviceId: "macbook", machine: "MacBook Air", panes: [expect.objectContaining({ label: "try" }), expect.objectContaining({ label: "shop-api-2" })] }]);
    // A tab strip holds the same machine's panes of the same place.
    const loose = panes.find((pane) => pane.label === "try")!;
    expect(guestTabs(panes, loose).map((pane) => pane.label)).toEqual(["try", "shop-api-2"]);
  });

  it("does only the closed set to another machine's pane (D-74)", () => {
    for (const command of ["split_right", "split_down", "new_tab", "rename_tab", "start_agent"]) expect(guestPaneRefuses(command)).toBe(true);
    for (const command of ["close_pane", "close_tab", "search", "text_larger"]) expect(guestPaneRefuses(command)).toBe(false);
    expect(isGuestPaneId("guest:macbook:pane:w1:p1")).toBe(true);
    expect(isGuestPaneId("remote:macbook:pane:w1:p1")).toBe(false);
    expect(isGuestPaneId("w1:p1")).toBe(false);
  });
});

describe("a connected machine's own Needs You and Done (PRD device-connect-iroh D-50, B49)", () => {
  const told = (rows: { id: string; group: "needs_you" | "done" }[]): SnapshotRest =>
    ({
      navigator: {
        peers: [{ key: KEY, device_id: "windows", name: "Windows PC", connected: true }],
        peer_work: [{ device_id: "windows", key: KEY, needs_you: 1, done: 1, rows: rows.map((row) => ({ ...row, title: row.id, agent_kind: "claude", mark: "turn", status_code: "question" })) }],
      },
    }) as SnapshotRest;

  it("lists each row under its own state with its machine's name", () => {
    const state = told([{ id: "w1:p1", group: "needs_you" }, { id: "w1:p2", group: "done" }]);
    expect(peerRows(state, "needs_you").map(({ row, machine, deviceId }) => [row.id, machine, deviceId])).toEqual([["w1:p1", "Windows PC", "windows"]]);
    expect(peerRows(state, "done").map(({ row }) => row.id)).toEqual(["w1:p2"]);
  });

  it("lists nothing of a machine that told nothing", () => {
    expect(peerRows({ navigator: {} } as SnapshotRest, "needs_you")).toEqual([]);
    expect(peerRows(null, "done")).toEqual([]);
  });
});
