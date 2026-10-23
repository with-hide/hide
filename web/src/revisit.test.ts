import { describe, expect, it } from "vitest";
import { errorNotice } from "./errorNotice";
import { paneMenuItems } from "./PaneRelations";
import type { TaskCardScope, TaskState } from "./agentScope";
import type { AgentRow, PaneRow, Workspace } from "./snapshot";
import { taskList } from "./taskList";

// PRD revisit-group: 다시 볼 것 stands under 읽을 것 in both views, and the pane
// menu offers the ★ or 다시 볼 것에서 빼기 on a root agent.

const project = { id: "p", label: "repo", device_id: "local", path: "/repo", checkouts: [{ id: "c", path: "/repo" }] } as unknown as Workspace;
const agent = (pane: string): AgentRow => ({ pane_id: pane, identity_label: pane, agent_kind: "claude" }) as unknown as AgentRow;
const card = (root: string, state: TaskState): TaskCardScope => ({ root, state, members: [root], checkouts: ["c"], recency: "" }) as unknown as TaskCardScope;

describe("다시 볼 것", () => {
  const cards = [card("ask", "needs_you"), card("work", "working"), card("later", "revisit"), card("read", "done"), card("wait", "waiting")];
  const rows = new Map(cards.map((c) => [c.root, agent(c.root)]));

  it("stands between 읽을 것 and 작업 중 in 상태별 (B1)", () => {
    const list = taskList({ cards } as never, rows, [project], "status");
    expect(list.groups.map((group) => group.state)).toEqual(["needs_you", "done", "revisit", "working", "waiting"]);
    expect(list.total).toBe(5);
  });

  it("is a band in the same order inside a project in 프로젝트별 (B14)", () => {
    const list = taskList({ cards } as never, rows, [project], "project");
    expect(list.groups[0]?.bands.map((band) => band.state)).toEqual(["needs_you", "done", "revisit", "working", "waiting"]);
    expect(list.groups[0]?.counts.revisit).toBe(1);
  });

  it("is not drawn when empty (B1)", () => {
    const list = taskList({ cards: [card("work", "working")] } as never, rows, [project], "status");
    expect(list.groups.map((group) => group.state)).toEqual(["working"]);
  });

  it("puts the ★ or 다시 볼 것에서 빼기 in the pane menu before 세션 보관 (B10)", () => {
    const pane = { id: "w1:p1", sleep_action: null, fork: null } as unknown as PaneRow;
    const star = paneMenuItems(pane, "title", { kind: "offered", unavailable: null }, "star").map((item) => item.id);
    expect(star.indexOf("revisit")).toBeLessThan(star.indexOf("archive_session"));
    expect(paneMenuItems(pane, "title", { kind: "offered", unavailable: null }, "done").find((item) => item.id === "revisit")?.label).toBe("Done revisiting");
    expect(paneMenuItems(pane, "title").some((item) => item.id === "revisit")).toBe(false);
  });

  it("says once when a change did not hold (B22, B23)", () => {
    expect(errorNotice({ kind: "revisit.save_failed", message: "x" })?.text).toBe("The revisit mark could not be saved. Try again.");
    expect(errorNotice({ kind: "revisit.capacity", message: "x" })?.text).toBe("Up to 256 tasks can wait in Revisit.");
  });
});
