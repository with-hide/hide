import { allAgents, deviceListedAgents, agentPlaces } from "../test/legacyNavigation";
import { projectRows } from "../test/legacyAgentScope";
import { agentSections } from "../test/legacyAgentScope";
import { emptyScope } from "../test/legacyAgentScope";
import { legacyAgentRow } from "../test/legacyAgentRow";
import { describe, expect, it } from "vitest";
import { allProjectsCount, boardProjects, mainSections } from "./navigation";

import { contextAllWorkspaces, contextWorkspaces, projectsOf } from "./remote";
import type { AgentRow, SnapshotRest, Workspace } from "./snapshot";

function agent(paneId: string, group: string): AgentRow {
  return legacyAgentRow({ id: paneId, pane_id: paneId, identity_label: paneId, agent_kind: "claude", mark: "turn", group, status_code: "idle", changed_at_unix_ms: null, emphasized: false, unread: false, demand: "none", activity: "idle" }) as AgentRow;
}

function workspace(id: string, deviceId: string, extra: Partial<Workspace> = {}, panes: string[] = []): Workspace {
  return { agent_scope: emptyScope(),
    id,
    label: id,
    path: `/${id}`,
    device_id: deviceId,
    registered: true,
    temporary: false,
    pinned: false,
    inactive_checkouts: { expanded: false, checkout_ids: [] },
    checkouts: [{ agent_scope: emptyScope(), id: `${id}:c`, workspace_id: id, label: "main", path: `/${id}`, tabs: [{ id: `${id}:t`, panes: panes.map((pane) => ({ id: pane })) }] }],
    ...extra,
  } as unknown as Workspace;
}

const LOCAL_HOME = workspace("home", "local", { is_home: true, label: "hide" }, ["h1"]);
const LOCAL_A = workspace("a", "local", {}, ["p1"]);
const MINI_HOME = workspace("remote:mini:home", "mini", { is_home: true, label: "hide" }, ["remote:mini:h"]);
const MINI_W = workspace("remote:mini:web", "mini", {}, ["remote:mini:p1"]);

const REST = {
  navigator: {
    focused_device_id: "local",
    devices: [
      { id: "local", label: "This Mac", kind: "local", state: "local", message: null },
      { id: "mini", label: "mini", kind: "remote", state: "ready", message: null },
    ],
    workspaces: [LOCAL_HOME, LOCAL_A],
  },
  ui_state: {
    workspace_registrations: [
      { id: "a", label: "a", path: "/a", device_id: "local", pinned: false },
      { id: "home", label: "hide", path: "/h", device_id: "local", pinned: false, home: true },
      { id: "remote:mini:web", label: "web", path: "/w", device_id: "mini", pinned: false },
    ],
  },
  status: {
    remote: [{ target_id: "mini", state: "connected", session: { agents: [agent("remote:mini:p1", "needs_you")], workspaces: [MINI_HOME, MINI_W] } }],
  },
} as unknown as SnapshotRest;

describe("the device's Home is no project (PRD home-device-rail D-13, B16)", () => {
  it("is left out of the Projects list and kept where it holds panes", () => {
    expect(contextWorkspaces(REST).map((row) => row.id)).toEqual(["a"]);
    expect(contextAllWorkspaces(REST).map((row) => row.id)).toEqual(["home", "a"]);
    // A list with no Home is returned as it is, and a list with one filters once per source.
    const plain = [LOCAL_A];
    expect(projectsOf(plain)).toBe(plain);
    const withHome = [LOCAL_HOME, LOCAL_A];
    expect(projectsOf(withHome)).toBe(projectsOf(withHome));
  });

  it("is not counted by an Overview, listed on it, or drawn on a board", () => {
    expect(allProjectsCount(REST, "local")).toBe(1);
    expect(allProjectsCount(REST, "mini")).toBe(1);
    expect(allProjectsCount(REST)).toBe(2);
    expect(mainSections(REST, "local").map((section) => section.device.id)).toEqual(["local"]);
    expect(mainSections(REST).map((section) => section.device.id)).toEqual(["local", "mini"]);
    expect(boardProjects(REST, [], "mini").map((row) => row.workspace.id)).toEqual(["remote:mini:web"]);
    expect(boardProjects(REST, []).map((row) => row.workspace.id)).toEqual(["a", "remote:mini:web"]);
  });

  it("counts a device with no session from its registrations, Home excluded", () => {
    const down = { ...REST, status: { remote: [{ target_id: "mini", state: "not_connected", session: null }] } } as unknown as SnapshotRest;
    expect(allProjectsCount(down, "mini")).toBe(1);
  });

  it("raises a Home agent's Needs You beside the projects' and names its place Home", () => {
    const listed = allAgents(REST.status!.remote, REST.navigator!.devices, [agent("h1", "needs_you"), agent("p1", "needs_you")]);
    const rows = projectRows(contextWorkspaces(REST), {}, listed, contextAllWorkspaces(REST).find((row) => row.is_home) ?? null);
    const raised = rows.find((row) => row.kind === "raised");
    expect(raised && raised.kind === "raised" ? raised.agents.map((row) => row.agent.pane_id) : []).toEqual(["h1", "p1"]);
    const withoutHome = projectRows(contextWorkspaces(REST), {}, listed);
    const plain = withoutHome.find((row) => row.kind === "raised");
    expect(plain && plain.kind === "raised" ? plain.agents.map((row) => row.agent.pane_id) : []).toEqual(["p1"]);
    expect(agentPlaces(REST.navigator!.workspaces, REST.status!.remote, REST.navigator!.devices)(null, "h1")).toBe("Home");
  });
});

describe("a device's Agents tab lists only that device's agents by status (quick device-rail-badges B3)", () => {
  const local = [agent("l1", "working"), agent("l2", "needs_you")];

  it("groups Needs You, Done, Working, Seen for the device in front and names no other device's agent", () => {
    const here = deviceListedAgents(REST.status!.remote, REST.navigator!.devices, local, "local");
    expect(agentSections(here.map((row) => row.agent)).map((section) => [section.group, section.agents.map((row) => row.pane_id)])).toEqual([
      ["needs_you", ["l2"]],
      ["working", ["l1"]],
    ]);
    const there = deviceListedAgents(REST.status!.remote, REST.navigator!.devices, local, "mini");
    expect(there.map((row) => [row.agent.pane_id, row.device])).toEqual([["remote:mini:p1", "mini"]]);
  });

  it("leaves out a device that is not connected", () => {
    const down = [{ target_id: "mini", state: "stale", session: { agents: [agent("x", "needs_you")], workspaces: [] } }] as unknown as NonNullable<SnapshotRest["status"]>["remote"];
    expect(deviceListedAgents(down, REST.navigator!.devices, [], "mini")).toEqual([]);
    expect(allAgents(down, REST.navigator!.devices, [])).toEqual([]);
  });
});
