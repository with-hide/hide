import { emptyScope } from "../test/legacyAgentScope";
import { describe, expect, it } from "vitest";
import type { Checkout, Device, SnapshotRest, Workspace } from "./snapshot";
import { initializeInterfaceI18n } from "./i18n/instance";
import { checkoutKey, homeKey, resolveTarget, startTargets as targetsWith, worktreeKey } from "./startTargets";
import type { Screen } from "./ui";

const { t } = initializeInterfaceI18n("en");
const startTargets = (rest: SnapshotRest | null, screen: Screen | null, settings = false, projectId: string | null = null) => targetsWith(rest, screen, t, settings, projectId);

function checkout(id: string, workspaceId: string, path: string, over: Partial<Checkout> = {}): Checkout {
  return { id, workspace_id: workspaceId, label: path.split("/").pop() ?? id, path, exists: true, ...over } as Checkout;
}

function workspace(id: string, label: string, device: string, checkouts: Checkout[], over: Partial<Workspace> = {}): Workspace {
  return { agent_scope: emptyScope(), id, label, path: `/${label}`, device_id: device, checkouts, temporary: false, registered: true, ...over } as Workspace;
}

const device = (id: string, label: string, kind: "local" | "remote"): Device => ({ id, label, kind, state: kind === "local" ? "local" : "ready" }) as Device;

const HIDE_MAIN = checkout("c-main", "w-hide", "/hide", { is_primary: true, label: "main" });
const HIDE_TREE = checkout("c-tree", "w-hide", "/hide.worktrees/rail", { is_worktree: true, label: "rail" });
const SASU_MAIN = checkout("c-sasu", "w-sasu", "/sasu", { is_primary: true, label: "main" });
const MINI_MAIN = checkout("m-main", "mw", "/srv/app", { is_primary: true, label: "main" });

function rest(over: { front?: string; connected?: boolean; miniSession?: boolean; focusedCheckout?: string } = {}): SnapshotRest {
  const front = over.front ?? "local";
  const miniWorkspaces = [
    workspace("mw", "app", "mini", [MINI_MAIN], { remote_target_id: "mini" }),
    workspace("mh", "hide", "mini", [checkout("m-home", "mh", "/home/u/hide")], { is_home: true, remote_target_id: "mini" }),
  ];
  return {
    navigator: {
      focused_device_id: front,
      focused_checkout_id: over.focusedCheckout ?? "c-main",
      devices: [device("local", "This Mac", "local"), device("mini", "mini", "remote"), device("box", "build-box", "remote")],
      workspaces: [
        workspace("w-hide", "hide", "local", [HIDE_MAIN, HIDE_TREE]),
        workspace("w-sasu", "sasu", "local", [SASU_MAIN]),
        workspace("w-home", "hide", "local", [checkout("h1", "w-home", "/Users/example/hide")], { is_home: true }),
      ],
    },
    status: {
      remote: [
        {
          target_id: "mini",
          state: over.connected === false ? "not_connected" : "connected",
          message: null,
          session: over.miniSession === false ? null : { workspaces: miniWorkspaces, focused_checkout_id: "m-main" },
        },
        { target_id: "box", state: "not_connected", message: null, session: null },
      ],
    },
  } as unknown as SnapshotRest;
}

const overviewTargets = (snapshot: SnapshotRest, projectId: string) => startTargets(snapshot, { kind: "main" }, false, projectId);
const keys = (rest: SnapshotRest, screen: Screen | null) => startTargets(rest, screen).groups.map((group) => group.items.map((item) => item.key));

describe("start target default (PRD home-device-rail D-19, B25)", () => {
  it("is the checkout in front, a worktree included", () => {
    const workspaceScreen: Screen = { kind: "workspace" };
    expect(startTargets(rest(), workspaceScreen).defaultKey).toBe(checkoutKey("local", "/hide"));
    expect(startTargets(rest({ focusedCheckout: "c-tree" }), workspaceScreen).defaultKey).toBe(checkoutKey("local", "/hide.worktrees/rail"));
  });

  it("is a project Overview's main checkout", () => {
    expect(overviewTargets(rest(), "w-sasu").defaultKey).toBe(checkoutKey("local", "/sasu"));
    expect(overviewTargets(rest(), "w-hide").defaultKey).toBe(checkoutKey("local", "/hide"));
  });

  it("is the front device's Home when no project is in front", () => {
    for (const screen of [null, { kind: "main" } as Screen]) expect(startTargets(rest(), screen).defaultKey).toBe(homeKey("local"));
    expect(startTargets(rest({ front: "mini" }), { kind: "main" }).defaultKey).toBe(homeKey("mini"));
  });

  it("is that device's own checkout when a remote device's thing is in front", () => {
    expect(startTargets(rest({ front: "mini", focusedCheckout: "m-main" }), { kind: "workspace" }).defaultKey).toBe(checkoutKey("mini", "/srv/app"));
    expect(overviewTargets(rest({ front: "mini" }), "mw").defaultKey).toBe(checkoutKey("mini", "/srv/app"));
  });

  it("is the front device's Home when the panel took Settings' place, whatever is under it", () => {
    expect(startTargets(rest(), { kind: "workspace" }, true).defaultKey).toBe(homeKey("local"));
    expect(startTargets(rest({ front: "mini", focusedCheckout: "m-main" }), { kind: "workspace" }, true).defaultKey).toBe(homeKey("mini"));
  });

  it("is the Home of the device a Home screen names", () => {
    expect(startTargets(rest(), { kind: "main", deviceId: "mini" }).groups[0]?.deviceId).toBe("mini");
    expect(startTargets(rest(), { kind: "main", deviceId: "mini" }).defaultKey).toBe(homeKey("mini"));
  });

  it("reads a checkout of the Home itself as the Home, since Home is not a project", () => {
    expect(startTargets(rest({ focusedCheckout: "h1" }), { kind: "workspace" }).defaultKey).toBe(homeKey("local"));
    expect(overviewTargets(rest(), "w-home").defaultKey).toBe(homeKey("local"));
  });

  it("falls back to Home when the thing in front is not listed", () => {
    expect(overviewTargets(rest(), "gone").defaultKey).toBe(homeKey("local"));
  });

  it("falls back to the first usable target when the front device is not connected", () => {
    const down = rest({ front: "mini", connected: false });
    expect(startTargets(down, { kind: "main" }).defaultKey).toBe(homeKey("local"));
  });
});

describe("start target menu (B26)", () => {
  it("lists the front device's Home and checkouts, then each other device's Home and checkouts", () => {
    expect(keys(rest(), { kind: "main" })).toEqual([
      [homeKey("local"), checkoutKey("local", "/hide"), checkoutKey("local", "/hide.worktrees/rail"), checkoutKey("local", "/sasu")],
      [homeKey("mini"), checkoutKey("mini", "/srv/app")],
      [homeKey("box")],
    ]);
  });

  it("puts a remote front device first", () => {
    const groups = startTargets(rest({ front: "mini" }), { kind: "main" }).groups;
    expect(groups.map((group) => group.deviceId)).toEqual(["mini", "local", "box"]);
  });

  it("never lists the Home workspace among checkouts", () => {
    const all = startTargets(rest(), null).groups.flatMap((group) => group.items);
    expect(all.some((item) => item.kind === "checkout" && ["w-home", "mh"].includes(item.workspaceId))).toBe(false);
  });

  it("names the front device's items plainly and the others' with their device", () => {
    const labels = startTargets(rest(), null).groups.map((group) => group.items.map((item) => item.label));
    expect(labels[0]).toEqual(["Home", "hide · main", "hide · rail", "sasu · main"]);
    expect(labels[1]).toEqual(["mini · Home", "mini · app · main"]);
    // Home is the one word this module words itself.
    const ko = targetsWith(rest(), null, initializeInterfaceI18n("ko").t).groups.map((group) => group.items[0]!.label);
    expect(ko).toEqual(["홈", "mini · 홈", "build-box · 홈"]);
  });

  it("disables every item of a device that is not connected, with the reason", () => {
    const all = startTargets(rest({ connected: false }), null).groups;
    const mini = all.find((group) => group.deviceId === "mini")!;
    expect(mini.connected).toBe(false);
    expect(mini.items.every((item) => item.disabled === "not_connected")).toBe(true);
    expect(all[0]!.items.every((item) => item.disabled === null)).toBe(true);
    // A device that never answered still has its Home, disabled.
    const box = all.find((group) => group.deviceId === "box")!;
    expect(box.items.map((item) => [item.key, item.disabled])).toEqual([[homeKey("box"), "not_connected"]]);
  });

  it("offers nothing before the first snapshot", () => {
    expect(startTargets(null, null)).toEqual({ groups: [], defaultKey: null });
  });
});

describe("resolving the target a start goes to", () => {
  it("uses the confirmed scoped worktree default without changing explicit or global targets", () => {
    const snapshot = rest();
    snapshot.navigator!.workspaces![0]!.is_git = true;
    snapshot.ui_state = { project_settings: {
      unrelated: { version: 1, device_id: "mini", root: "/hide", new_work: "checkout" },
      selected: { version: 1, device_id: "local", root: "/hide", new_work: "worktree" },
    } } as SnapshotRest["ui_state"];
    const scoped = overviewTargets(snapshot, "w-hide");
    expect(scoped.defaultKey).toBe(worktreeKey("local", "/hide"));
    expect(resolveTarget(scoped, checkoutKey("local", "/hide.worktrees/rail"))?.key).toBe(checkoutKey("local", "/hide.worktrees/rail"));
    expect(startTargets(snapshot, { kind: "workspace" }).defaultKey).toBe(checkoutKey("local", "/hide"));
    expect(startTargets(snapshot, { kind: "workspace" }, true, "w-hide").defaultKey).toBe(homeKey("local"));
    snapshot.ui_state!.project_settings!.selected!.new_work = "checkout";
    expect(overviewTargets(snapshot, "w-hide").defaultKey).toBe(checkoutKey("local", "/hide"));
  });
  it("keeps the operator's pick while it is listed and usable, else the default", () => {
    const targets = startTargets(rest(), { kind: "main" });
    expect(resolveTarget(targets, checkoutKey("local", "/sasu"))?.key).toBe(checkoutKey("local", "/sasu"));
    expect(resolveTarget(targets, checkoutKey("local", "/removed"))?.key).toBe(homeKey("local"));
    expect(resolveTarget(targets, homeKey("box"))?.key).toBe(homeKey("local"));
    expect(resolveTarget(targets, null)?.key).toBe(homeKey("local"));
  });
});
