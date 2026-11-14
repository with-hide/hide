// PRD hide-external-control B40, D-48: a tool's cell draws the tool's work
// only, even in a project the operator and another tool work in too.
import { expect, it } from "vitest";
import { emptyScope } from "../test/legacyAgentScope";
import type { ToolCell } from "./agentScope";
import { projectRows } from "./overviewProjects";
import type { BoardProject } from "./projectBoard";
import type { Workspace } from "./snapshot";
import { toolProjects } from "./toolCells";

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
