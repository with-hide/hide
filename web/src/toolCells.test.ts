// PRD hide-external-control B40, D-48: a tool's cell draws the tool's work
// only, even in a project the operator and another tool work in too, and is
// in front while the center shows the tool's page or a pane it keeps.
import { expect, it } from "vitest";
import { emptyScope } from "../test/legacyAgentScope";
import type { ToolCell } from "./agentScope";
import { projectRows } from "./overviewProjects";
import type { BoardProject } from "./projectBoard";
import type { SnapshotRest, Workspace } from "./snapshot";
import { frontTool, toolProjects } from "./toolCells";
import type { Screen } from "./ui";

const card = (root: string, state: "needs_you" | "working") => ({ root, state, members: [root], checkouts: ["shared-main"], recency: "1" });
const member = (pane: string, project = "shared") => ({ pane_id: pane, project_id: project, checkout_id: `${project}-main` });
const project = (id: string, cards: ReturnType<typeof card>[]): BoardProject => ({
  agents: [],
  device: null,
  workspace: { id, label: id, device_id: "local", path: `/fixture/${id}`, checkouts: [], agent_scope: { ...emptyScope(), members: cards.map((row) => member(row.root, id)), tasks: { cards, needs_you: cards.filter((row) => row.state === "needs_you").length, recency: "1" } } } as unknown as Workspace,
});
const cell = (connection: string, cards: ReturnType<typeof card>[]): ToolCell => ({ connection, name: connection, turn: 0, devices: [{ device_id: "local", tasks: { cards, needs_you: 0, recency: "1" }, members: cards.map((row) => member(row.root)) }] });

it("counts a tool's own tasks in a project it shares with the operator and another tool", () => {
  // The core keeps the tools' work out of the project's own scope, so it holds the operator's question alone.
  const projects = [project("shared", [card("operator-pane", "needs_you")]), project("elsewhere", [card("other-pane", "working")])];
  const nightly = cell("nightly", [card("nightly-pane", "working")]);
  const weekly = cell("weekly", [card("weekly-pane", "needs_you")]);
  const now = (rows: BoardProject[]) => projectRows(rows).map((row) => [row.project.id, row.now.needs, row.now.working, row.now.total]);
  expect(now(toolProjects(projects, nightly)), "nightly").toEqual([["shared", 0, 1, 1]]);
  expect(now(toolProjects(projects, weekly)), "weekly").toEqual([["shared", 1, 0, 1]]);
  expect(toolProjects(projects, nightly)[0]?.workspace.agent_scope.members.map((row) => row.pane_id), "the graph's members").toEqual(["nightly-pane"]);
  expect(now(projects), "the device's own").toEqual([["shared", 1, 0, 1], ["elsewhere", 0, 1, 1]]);
});

// A tool's cell is a scope like a device's (PRD hide-external-control B40, B44; operator decision 2026-11-15):
// which one is in front is read from what the center shows, never kept beside it.
const MINI = "mini";
const scoped = (focusedPane: string | null, tools: ToolCell[], focusedDevice = "local"): SnapshotRest => ({
  navigator: { focused_device_id: focusedDevice, devices: [{ id: "local", kind: "local" }, { id: MINI, kind: "remote" }], workspaces: [], tools },
  focused: { pane_id: focusedPane },
  status: { remote: [] },
}) as unknown as SnapshotRest;
const nightly: ToolCell = { connection: "nightly", name: "nightly", turn: 0, devices: [{ device_id: "local", tasks: { cards: [], needs_you: 0, recency: "1" }, members: [member("tool-pane")] }] };

it("puts a tool's cell in front on its Overview page and over a pane it keeps, and the device's otherwise", () => {
  const rest = scoped("tool-pane", [nightly]);
  const workspace: Screen = { kind: "workspace" };
  expect(frontTool(rest, { kind: "main", tool: "nightly" })?.connection, "its tile's Overview page").toBe("nightly");
  expect(frontTool(rest, { kind: "missions", tool: "nightly" })?.connection, "its mission screen").toBe("nightly");
  expect(frontTool(rest, workspace)?.connection, "its agent's pane in front").toBe("nightly");
  expect(frontTool(scoped("operator-pane", [nightly]), workspace), "the operator's pane").toBeNull();
  expect(frontTool(rest, { kind: "main" }), "the device's Overview").toBeNull();
  expect(frontTool(rest, { kind: "missions" }), "the device's mission screen").toBeNull();
  expect(frontTool(rest, { kind: "desk" }), "Desk").toBeNull();
  // The pane id names a pane of the device in front only: the same id on another machine is not the tool's.
  expect(frontTool(scoped("tool-pane", [nightly], MINI), workspace), "another device in front").toBeNull();
});

it("puts the device in front once the tool's cell goes, on its page and over its pane alike", () => {
  const gone = scoped("tool-pane", []);
  expect(frontTool(gone, { kind: "main", tool: "nightly" })).toBeNull();
  expect(frontTool(gone, { kind: "workspace" })).toBeNull();
  // Let go of at View only, the pane is still there but no cell keeps it.
  expect(frontTool(scoped("tool-pane", [{ ...nightly, devices: [] }]), { kind: "workspace" })).toBeNull();
});
