import { describe, expect, it } from "vitest";
import type { DeskItem } from "./desk";
import { deskFileScope, deskTabOf, lastNamedWorkspace, preferredRoot, projectFileRows, type DeskFileHit } from "./deskPalette";
import type { SearchEntry } from "./search";
import type { Checkout, DeskMention, SnapshotRest } from "./snapshot";

// Expected answers are PRD desk-2-ui D-23, D-24, B37-B39: ⌘P on Desk searches the
// checkout of the session beside Desk, else every project, the project Desk
// named last first; a ⌘K PR or mission becomes a tab of the tab panel.

const checkout = (id: string, path: string, pane: string): Checkout => ({
  id, workspace_id: `w-${id}`, path, label: id, branch: null, purpose: null, is_worktree: false, exists: true, has_panes: true, pull_request: null, agent_scope: undefined as never, next_tab_label: "Tab 2",
  tabs: [{ id: `t-${id}`, workspace_id: `w-${id}`, checkout_id: id, label: "Tab 1", empty: false, delegated: false, panes: [{ id: pane, device_id: "local", device_pane_id: pane, cwd: path, engine_label: null, terminal_title: null, identity_label: null, status_code: "idle", requires_close_confirmation: false, requires_close_status_check: false }] }], active_tab_id: `t-${id}`, strip: [],
});
const workspace = (one: Checkout, label: string) => ({ id: one.workspace_id, label, device_id: "local", path: one.path, registered: true, temporary: false, pinned: false, inactive_checkouts: { expanded: false, checkout_ids: [] }, agent_scope: undefined as never, checkouts: [one] });
const rest = {
  navigator: { focused_device_id: "local", devices: [{ id: "local", kind: "local" }], workspaces: [workspace(checkout("ide", "/ide", "p-ide"), "herdr-ide"), workspace(checkout("atorni", "/atorni", "p-atorni"), "atorni-ax")] },
  ui_state: {},
} as unknown as SnapshotRest;

const hit = (root: string, relative: string): DeskFileHit => ({ device_id: "local", root_path: root, path: `${root}/${relative}`, relative_path: relative });
const user = (id: number, mentions: DeskMention[]): DeskItem => ({ kind: "user", id, text: "", at_ms: id, mentions });

describe("where ⌘P searches on Desk", () => {
  it("searches the checkout of the session beside Desk", () => {
    expect(deskFileScope(rest, "p-atorni")).toEqual({ kind: "checkout", device: "local", root: "/atorni" });
  });
  it("searches every project with no session open, or with one whose checkout is no longer listed", () => {
    expect(deskFileScope(rest, null)).toEqual({ kind: "all" });
    expect(deskFileScope(rest, "p-gone")).toEqual({ kind: "all" });
  });
});

describe("the project Desk named last", () => {
  it("is the newest line's project, by a project mention (its checkout's path) or by a session's pane", () => {
    expect(lastNamedWorkspace(rest, [user(1, [{ kind: "project", id: "/ide", name: "herdr-ide" }]), user(2, [{ kind: "project", id: "/atorni", name: "atorni-ax" }])])).toBe("w-atorni");
    expect(lastNamedWorkspace(rest, [user(1, [{ kind: "project", id: "/atorni", name: "atorni-ax" }]), user(2, [{ kind: "session", id: "p-ide", name: "x" }])])).toBe("w-ide");
  });
  it("is none when no line names one, or the named one is not listed", () => {
    expect(lastNamedWorkspace(rest, [user(1, [])])).toBeNull();
    expect(lastNamedWorkspace(rest, [user(1, [{ kind: "project", id: "/gone", name: "gone" }])])).toBeNull();
  });
});

describe("the project the core is asked to put first", () => {
  it("is the named project's path on its device, and none without one", () => {
    expect(preferredRoot(rest, "w-ide")).toEqual({ device_id: "local", root_path: "/ide" });
    expect(preferredRoot(rest, null)).toBeNull();
    expect(preferredRoot(rest, "w-gone")).toBeNull();
  });
});

describe("the all-projects answer", () => {
  it("puts the named project's files first, then the others, each in hided's rank, with the project and folder split off the name", () => {
    const files = [hit("/atorni", "src/auth/token-guard.ts"), hit("/ide", "web/src/token.ts"), hit("/atorni", "token.test.ts"), hit("/ide", "docs/token-refresh.md")];
    const rows = projectFileRows(rest, files, "w-ide");
    expect(rows.named.map((row) => `${row.project} › ${row.name} ${row.folder}`)).toEqual(["herdr-ide › token.ts web/src", "herdr-ide › token-refresh.md docs"]);
    expect(rows.other.map((row) => `${row.project} › ${row.name} ${row.folder}`)).toEqual(["atorni-ax › token-guard.ts src/auth", "atorni-ax › token.test.ts "]);
  });
  it("leaves out a file of a checkout that is not listed", () => {
    expect(projectFileRows(rest, [hit("/gone", "a.ts")], null)).toEqual({ named: [], other: [] });
  });
});

describe("what ⌘K opens in the tab panel", () => {
  const base = { id: "x", title: "t", subtitle: "", group: { id: "g", label: "kind.mission" } } as const;
  it("opens a mission as its map and a PR as an output tab, and nothing else", () => {
    expect(deskTabOf({ ...base, kind: "mission", missionId: "mission-a", title: "1.3.0" } as SearchEntry)).toEqual({ kind: "mission", mission: "mission-a", title: "1.3.0" });
    const tab = deskTabOf({ ...base, kind: "pr", workspaceId: "w-ide", pr: { number: 12345, state: "mergeable", url: "https://github.com/o/r/pull/12345" } } as unknown as SearchEntry);
    expect(tab).toMatchObject({ kind: "output", output: { id: "pr:w-ide:12345", kind: "pr", label: "#12345", pr_state: "mergeable", url: "https://github.com/o/r/pull/12345" } });
    expect(deskTabOf({ ...base, kind: "agent" } as SearchEntry)).toBeNull();
    expect(deskTabOf({ ...base, kind: "pr", external: true, workspaceId: "w-ide", pr: { number: 1 } } as unknown as SearchEntry)).toBeNull();
  });
});
