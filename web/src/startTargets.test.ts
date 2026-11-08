import { emptyScope } from "../test/legacyAgentScope";
import { describe, expect, it } from "vitest";
import type { Checkout, Device, SnapshotRest, Workspace } from "./snapshot";
import { initializeInterfaceI18n } from "./i18n/instance";
import { checkoutKey, projectKey, projectOf, resolveTarget, startTargets as targetsWith, worktreeKey } from "./startTargets";
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
  ];
  return {
    navigator: {
      focused_device_id: front,
      focused_checkout_id: over.focusedCheckout ?? "c-main",
      devices: [device("local", "This Mac", "local"), device("mini", "mini", "remote"), device("box", "build-box", "remote")],
      workspaces: [
        workspace("w-hide", "hide", "local", [HIDE_MAIN, HIDE_TREE]),
        workspace("w-sasu", "sasu", "local", [SASU_MAIN]),
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

  it("is the front device's first project when no project is in front and none was used", () => {
    for (const screen of [null, { kind: "main" } as Screen]) expect(startTargets(rest(), screen).defaultKey).toBe(checkoutKey("local", "/hide"));
    expect(startTargets(rest({ front: "mini" }), { kind: "main" }).defaultKey).toBe(checkoutKey("mini", "/srv/app"));
  });

  it("is the front device's most recently used project when no project is in front (PRD retire-hide-home B2)", () => {
    const used = rest();
    used.ui_state = { recent_checkouts: [{ device_id: "mini", checkout_id: "m-main", project_name: "app", branch: "main" }, { device_id: "local", checkout_id: "c-sasu", project_name: "sasu", branch: "main" }, { device_id: "local", checkout_id: "c-main", project_name: "hide", branch: "main" }] } as SnapshotRest["ui_state"];
    expect(startTargets(used, { kind: "main" }).defaultKey).toBe(checkoutKey("local", "/sasu"));
    expect(startTargets(used, { kind: "workspace" }, true).defaultKey).toBe(checkoutKey("local", "/sasu"));
    expect(startTargets(used, { kind: "main", deviceId: "mini" }).defaultKey).toBe(checkoutKey("mini", "/srv/app"));
  });

  it("is that device's own checkout when a remote device's thing is in front", () => {
    expect(startTargets(rest({ front: "mini", focusedCheckout: "m-main" }), { kind: "workspace" }).defaultKey).toBe(checkoutKey("mini", "/srv/app"));
    expect(overviewTargets(rest({ front: "mini" }), "mw").defaultKey).toBe(checkoutKey("mini", "/srv/app"));
  });

  it("is the front device's project when the panel took Settings' place, whatever is under it", () => {
    expect(startTargets(rest(), { kind: "workspace" }, true).defaultKey).toBe(checkoutKey("local", "/hide"));
    expect(startTargets(rest({ front: "mini", focusedCheckout: "m-main" }), { kind: "workspace" }, true).defaultKey).toBe(checkoutKey("mini", "/srv/app"));
  });

  it("is the project of the device a device screen names", () => {
    expect(startTargets(rest(), { kind: "main", deviceId: "mini" }).groups[0]?.deviceId).toBe("mini");
    expect(startTargets(rest(), { kind: "main", deviceId: "mini" }).defaultKey).toBe(checkoutKey("mini", "/srv/app"));
  });

  it("has no target, and names the front device for Add a project, when that device has no project (B1)", () => {
    const empty = rest();
    empty.navigator!.workspaces = [];
    const targets = startTargets(empty, { kind: "main" });
    expect(targets.defaultKey).toBeNull();
    expect(targets.frontDeviceId).toBe("local");
    expect(resolveTarget(targets, null)).toBeNull();
  });

  it("falls back to the front device's project when the thing in front is not listed", () => {
    expect(overviewTargets(rest(), "gone").defaultKey).toBe(checkoutKey("local", "/hide"));
  });

  it("falls back to the first usable target when the front device is not connected", () => {
    const down = rest({ front: "mini", connected: false });
    expect(startTargets(down, { kind: "main" }).defaultKey).toBe(checkoutKey("local", "/hide"));
  });
});

describe("start target menu (B26)", () => {
  it("lists the front device's checkouts, then each other device's, and no Home on any", () => {
    expect(keys(rest(), { kind: "main" })).toEqual([
      [checkoutKey("local", "/hide"), checkoutKey("local", "/hide.worktrees/rail"), checkoutKey("local", "/sasu")],
      [checkoutKey("mini", "/srv/app")],
      [],
    ]);
  });

  it("puts a remote front device first", () => {
    const groups = startTargets(rest({ front: "mini" }), { kind: "main" }).groups;
    expect(groups.map((group) => group.deviceId)).toEqual(["mini", "local", "box"]);
  });

  it("names the front device's items plainly and the others' with their device", () => {
    const labels = startTargets(rest(), null).groups.map((group) => group.items.map((item) => item.label));
    expect(labels[0]).toEqual(["hide · main", "hide · rail", "sasu · main"]);
    expect(labels[1]).toEqual(["mini · app · main"]);
  });

  it("disables every item of a device that is not connected, with the reason", () => {
    const all = startTargets(rest({ connected: false }), null).groups;
    const mini = all.find((group) => group.deviceId === "mini")!;
    expect(mini.connected).toBe(false);
    expect(mini.items.every((item) => item.disabled === "not_connected")).toBe(true);
    expect(all[0]!.items.every((item) => item.disabled === null)).toBe(true);
    // A device that never answered lists nothing to start in.
    const box = all.find((group) => group.deviceId === "box")!;
    expect([box.connected, box.items]).toEqual([false, []]);
  });

  it("offers nothing before the first snapshot", () => {
    expect(startTargets(null, null)).toEqual({ groups: [], frontDeviceId: null, defaultKey: null, currentKey: null });
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
    expect(startTargets(snapshot, { kind: "workspace" }, true, "w-hide").defaultKey).toBe(checkoutKey("local", "/hide"));
    snapshot.ui_state!.project_settings!.selected!.new_work = "checkout";
    expect(overviewTargets(snapshot, "w-hide").defaultKey).toBe(checkoutKey("local", "/hide"));
  });
  it("keeps the operator's pick while it is listed, else the default", () => {
    const targets = startTargets(rest(), { kind: "main" });
    expect(resolveTarget(targets, checkoutKey("local", "/sasu"))?.key).toBe(checkoutKey("local", "/sasu"));
    expect(resolveTarget(targets, checkoutKey("local", "/removed"))?.key).toBe(checkoutKey("local", "/hide"));
    expect(resolveTarget(targets, null)?.key).toBe(checkoutKey("local", "/hide"));
  });

  it("keeps a pick whose device stopped answering, disabled, instead of moving the start elsewhere", () => {
    const picked = checkoutKey("mini", "/srv/app");
    expect(resolveTarget(startTargets(rest(), { kind: "main" }), picked)?.disabled).toBeNull();
    const after = resolveTarget(startTargets(rest({ connected: false }), { kind: "main" }), picked);
    expect([after?.key, after?.disabled]).toEqual([picked, "not_connected"]);
  });
});

describe("the where row: which project, then which branch", () => {
  it("lists each device's projects, a project once however many checkouts it has", () => {
    const groups = startTargets(rest(), { kind: "main" }).groups;
    expect(groups.map((group) => group.projects.map((project) => project.label))).toEqual([["hide", "sasu"], ["app"], []]);
    expect(groups[0]!.projects[0]!.key).toBe(projectKey("local", "w-hide"));
  });

  it("puts the main checkout first and the one in front second, and marks the one in front", () => {
    const snapshot = rest({ focusedCheckout: "c-tree" });
    const extra = checkout("c-other", "w-hide", "/hide.worktrees/docs", { is_worktree: true, label: "docs", branch: "docs/start" });
    snapshot.navigator!.workspaces![0]!.checkouts = [extra, HIDE_TREE, HIDE_MAIN];
    const targets = startTargets(snapshot, { kind: "workspace" });
    const hide = targets.groups[0]!.projects[0]!;
    expect(hide.branches.map((branch) => branch.key)).toEqual([checkoutKey("local", "/hide"), checkoutKey("local", "/hide.worktrees/rail"), checkoutKey("local", "/hide.worktrees/docs")]);
    expect(hide.branches.map((branch) => branch.kind === "checkout" && branch.branch)).toEqual(["main", "rail", "docs/start"]);
    expect(targets.currentKey).toBe(checkoutKey("local", "/hide.worktrees/rail"));
    expect(targets.defaultKey).toBe(targets.currentKey);
    // Only a Workspace in front has a checkout in front.
    expect(startTargets(snapshot, { kind: "main" }).currentKey).toBeNull();
  });

  it("offers a new branch for a Git project only, and names the project a target belongs to", () => {
    const snapshot = rest();
    snapshot.navigator!.workspaces![0]!.is_git = true;
    const targets = startTargets(snapshot, { kind: "main" });
    const [hide, sasu] = targets.groups[0]!.projects;
    expect(hide!.worktree?.key).toBe(worktreeKey("local", "/hide"));
    expect(sasu!.worktree).toBeNull();
    expect(projectOf(targets, resolveTarget(targets, worktreeKey("local", "/hide")))?.key).toBe(hide!.key);
    expect(projectOf(targets, resolveTarget(targets, checkoutKey("local", "/hide.worktrees/rail")))?.key).toBe(hide!.key);
  });
});
