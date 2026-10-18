import { describe, expect, it } from "vitest";
import { groupNameProblem } from "./groupTabs";
import { numberedTabs } from "./numbering";
import { sidebarNamedGroups, sidebarTabGroups } from "./sidebarTabs";
import { inlineStartKey } from "./startDraft";
import type { AgentLayout, AgentStartTarget } from "./agentLayout";
import type { Checkout, Tab } from "./snapshot";
import { legacyAgentRow } from "../test/legacyAgentRow";

describe("named groups (group-tabs B4, B12, B20, B25)", () => {
  it("accepts 80 Unicode characters and rejects controls, duplicate trimmed names and the 33rd group", () => {
    const groups = [{ id: null, name: null, tabs: [] }, { id: "g1", name: "개편", tabs: [] }];
    expect(groupNameProblem("😀".repeat(80), groups, null)).toBeNull();
    expect(groupNameProblem("😀".repeat(81), groups, null)).toBe("groups.invalidName");
    expect(groupNameProblem(" \n ", groups, null)).toBe("groups.invalidName");
    expect(groupNameProblem("line\nnext", groups, null)).toBe("groups.invalidName");
    expect(groupNameProblem("\u0000name", groups, null)).toBe("groups.invalidName");
    expect(groupNameProblem(" 개편 ", groups, null)).toBe("groups.duplicateName");
    expect(groupNameProblem("개편", groups, "g1")).toBeNull();
    const full = Array.from({ length: 32 }, (_, index) => ({ id: `g${index}`, name: String(index), tabs: [] }));
    expect(groupNameProblem("new", full, null)).toBe("groups.limit");
    expect(groupNameProblem("renamed", full, "g0")).toBeNull();
  });

  it("keeps delegated descendants with their root and exposes empty groups, terminals and drafts", () => {
    const agent = (id: string) => legacyAgentRow({ id, pane_id: id, identity_label: id, agent_kind: "codex", device_id: "local" });
    const root = agent("root"), child = agent("child");
    const tabs = [
      { id: "t1", panes: [{ id: "root" }] },
      { id: "t2", panes: [{ id: "child" }], delegated: true },
      { id: "terminal", panes: [{ id: "shell" }] },
    ] as Tab[];
    const checkout = { tabs, agent_groups: { selected: "work", drafts: ["draft-1"], groups: [
      { id: null, name: null, tabs: ["terminal"] },
      { id: "work", name: "작업", tabs: ["t1", "draft-1"] },
      { id: "empty", name: "준비", tabs: [] },
    ] } } as Checkout;
    const segments = sidebarTabGroups(checkout, "local", [{ agent: root, depth: 0 }, { agent: child, depth: 1 }], [root, child]);
    const groups = sidebarNamedGroups(checkout, segments);
    expect(groups.map(({ group }) => group.id)).toEqual([null, "work", "empty"]);
    expect(groups[0]!.segments.map(({ tab }) => tab?.id)).toEqual(["terminal"]);
    expect(groups[1]!.segments.flatMap(({ rows }) => rows.map(({ agent }) => agent.pane_id))).toEqual(["root", "child"]);
    expect(groups[1]!.drafts).toEqual(["draft-1"]);
    expect(groups[2]!.segments).toEqual([]);
  });

  it("numbers only the selected group's live and draft tabs in their drawn order", () => {
    const checkout = { strip: ["hidden", "visible"].map((id) => ({ id, source_id: id, kind: "herdr" })) } as Checkout;
    const layout: AgentLayout = { group_id: "work", drafts: ["draft-1"], root: { area: { id: "a1", active: "draft-1", displays: [{ id: "draft-1" }, { id: "visible" }] } }, active_area: "a1", canvases: {}, limits: { areas: 6, depth: 3, displays: 64 }, display_count: 2 };
    expect([...numberedTabs(checkout, layout)]).toEqual([[1, "draft-1"], [2, "visible"]]);
  });

  it("keeps inputs separate when two groups reuse area IDs or a tab offers add-agent", () => {
    const target: AgentStartTarget = { group_id: "g1", area_id: "a1", tab_id: null, add_pane: false };
    const key = inlineStartKey("local", "/fixture", target);
    expect(inlineStartKey("local", "/fixture", { ...target, group_id: "g2" })).not.toBe(key);
    expect(inlineStartKey("remote", "/fixture", target)).not.toBe(key);
    expect(inlineStartKey("local", "/other", target)).not.toBe(key);
    expect(inlineStartKey("local", "/fixture", { ...target, add_pane: true })).not.toBe(key);
  });
});
