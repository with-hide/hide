import { emptyScope } from "../test/legacyAgentScope";
import { legacyAgentRow } from "../test/legacyAgentRow";
import { describe, expect, it } from "vitest";
import type { AgentStatusCode, Checkout, TaskOperation, Workspace } from "./snapshot";
import * as manage from "./workspaceManage";
import { branchProblem, checkoutRemoving, discardConfirmationKey, factsLine, normalizePurpose, purposeIsLong, removalFor, scalarCount, taskFor } from "./workspaceManage";
import { initializeInterfaceI18n } from "./i18n/instance";

const { t } = initializeInterfaceI18n("en");
const ko = initializeInterfaceI18n("ko").t;
const agentMenu = (agent: Parameters<typeof manage.agentMenu>[0], chord: string) => manage.agentMenu(agent, chord, t);
const checkoutMenu = (workspace: Workspace, checkout: Checkout, host: Parameters<typeof manage.checkoutMenu>[2], purposeProblem: string | null = null) => manage.checkoutMenu(workspace, checkout, host, t, purposeProblem);
const folderMenu = (workspace: Workspace, checkout: Checkout, host: Parameters<typeof manage.folderMenu>[2], purposeProblem: string | null = null, issueSource?: string) =>
  manage.folderMenu(workspace, checkout, host, t, purposeProblem, issueSource);
const projectMenu = (workspace: Workspace, host: Parameters<typeof manage.projectMenu>[1], issueSource?: string) => manage.projectMenu(workspace, host, t, issueSource);
const deletionFacts = (checkout: Checkout, panes: number) => manage.deletionFacts(checkout, panes, t);
const projectRemovalFacts = (workspace: Workspace) => manage.projectRemovalFacts(workspace, t);
const purposeCountLabel = (text: string) => manage.purposeCountLabel(text, t);
const purposeScope = (device: string | null, branch: string | null) => manage.purposeScope(device, branch, t);
const remotePurposeProblem = (workspace: Workspace, remote: Parameters<typeof manage.remotePurposeProblem>[1]) => manage.remotePurposeProblem(workspace, remote, t);

const workspace = (patch: Partial<Workspace> = {}): Workspace => ({ agent_scope: emptyScope(),
  id: "w1",
  label: "hide",
  path: "/Users/example/hide",
  device_id: "local",
  remote_target_id: null,
  is_git: true,
  registered: true,
  temporary: false,
  pinned: false,
  checkouts: [],
  inactive_checkouts: { expanded: false, checkout_ids: [] },
  ...patch,
});

const checkout = (patch: Partial<Checkout> = {}): Checkout => ({ agent_scope: emptyScope(),
  id: "c1",
  workspace_id: "w1",
  label: "feature",
  path: "/Users/example/hide.worktrees/feature",
  branch: "feature",
  purpose: null,
  is_worktree: true,
  exists: true,
  has_panes: false,
  worktree: {
    path: "/Users/example/hide.worktrees/feature",
    branch: "feature",
    head_sha: "abc",
    is_main: false,
    missing: false,
    dirty: false,
    changed_file_count: 0,
    pane_count: 0,
    deletion_gate: { blocked: null, warnings: [], pane_count: 0, can_delete_branch: true, branch_loss: null, discard: null },
  },
  pull_request: null,
  tabs: [],
  active_tab_id: null,
  strip: [],
  next_tab_label: "1",
  ...patch,
});

const task = (patch: Partial<TaskOperation>): TaskOperation => ({
  id: 5,
  kind: "worktree_create",
  phase: "working",
  repository_root: "/Users/example/hide",
  branch: "feature",
  base_branch: null,
  path: null,
  pane_id: null,
  agent_kind: null,
  failure: null,
  agent_phase: null,
  agent_reason: null, agent_line: null,
  ...patch,
});

describe("purpose field", () => {
  it("counts and cuts Unicode scalars the way the core does, keeping one line", () => {
    const korean = "한".repeat(90);
    expect(scalarCount(normalizePurpose(korean))).toBe(80);
    expect(normalizePurpose("a\nb\rc")).toBe("a b c");
    expect(scalarCount("🙂🙂")).toBe(2);
    expect(purposeCountLabel("한글 목적")).toBe("5 / 40");
    expect(purposeIsLong("x".repeat(41))).toBe(true);
    expect(purposeIsLong("x".repeat(40))).toBe(false);
  });
});

describe("branch names", () => {
  it("refuses what Git would refuse before a request goes out", () => {
    expect(branchProblem("feature/s5")).toBeNull();
    for (const bad of ["", "-x", "a b", "a..b", "a~1", "x.lock", "/a", "a/", "a\u0001"]) expect(branchProblem(bad), bad).not.toBeNull();
  });
});

describe("row menus", () => {
  const desktop = { reveal: { label: "explorer.revealFinder" as const }, newTabChord: "⌘T", node: "local", screen: "local", editor: false };
  const browser = { reveal: null, newTabChord: "⌥T", node: "local", screen: "local", editor: false };
  /** Each item as the menu draws it: a separator line before it, its label, and its reason when disabled. */
  const drawn = (items: { label: string; separated?: boolean; unavailable: string | null; shortcut?: string }[]) =>
    items.flatMap((item) => [...(item.separated ? ["─"] : []), item.shortcut ? `${item.label} ${item.shortcut}` : item.label]);
  const primary = (patch: Partial<Checkout> = {}) => checkout({ id: "c0", label: "main", branch: "main", path: "/Users/example/hide", is_worktree: false, is_primary: true, ...patch });

  it("draws a project's menu in the board's order, the file manager only on the desktop (B1)", () => {
    const project = workspace({ checkouts: [primary(), checkout()] });
    expect(drawn(projectMenu(project, desktop))).toEqual([
      "Project settings",
      "New worktree…",
      "New tab in main ⌘T",
      "─",
      "Reveal in Finder",
      "Copy path",
      "─",
      "Pin",
      "Remove project…",
    ]);
    expect(drawn(projectMenu(project, browser))).toEqual(["Project settings", "New worktree…", "New tab in main ⌥T", "─", "Copy path", "─", "Pin", "Remove project…"]);
    expect(projectMenu(workspace({ pinned: true, checkouts: [primary()] }), desktop).find((item) => item.id === "unpin")?.label).toBe("Unpin");
    expect(projectMenu(workspace({ is_git: false, checkouts: [primary({ is_primary: false })] }), desktop).find((item) => item.id === "new_worktree")?.unavailable).toMatch(/not a Git/);
  });

  it("offers the Issue source choice on a project the boards read, checked as stored (B7, D-04)", () => {
    const source = { kind: "github", label: "GitHub", name: "acme/repo", reading: false, failure: null, last_read_at_unix_ms: null };
    const read = workspace({ checkouts: [primary()], tasks: { source, tasks: [] } as unknown as Workspace["tasks"] });
    const choices = (stored?: string) => projectMenu(read, desktop, stored).find((item) => item.id === "issue_source")?.choices?.map((choice) => [choice.id, choice.label, choice.checked]);
    expect(choices()).toEqual([
      ["issue_source_auto", "Automatic (GitHub)", true],
      ["issue_source_github", "GitHub · acme/repo", false],
      ["issue_source_local", "Local", false],
    ]);
    expect(choices("local")?.map((choice) => choice[2])).toEqual([false, false, true]);
    // A folder has no GitHub choice; a project without a source (a dialed device's) or the Home offers none, and a node's that dials the core offers it.
    const folder = workspace({ is_git: false, checkouts: [primary()], tasks: { source: { ...source, kind: "local", label: "Local" }, tasks: [] } as unknown as Workspace["tasks"] });
    expect(projectMenu(folder, desktop).find((item) => item.id === "issue_source")?.choices?.map((choice) => choice.id)).toEqual(["issue_source_auto", "issue_source_local"]);
    expect(projectMenu({ ...read, remote_target_id: "studio", device_id: "studio", tasks: { ...read.tasks!, source: null } }, desktop).some((item) => item.id === "issue_source")).toBe(false);
    expect(projectMenu({ ...read, remote_target_id: "macbook", device_id: "macbook" }, desktop).some((item) => item.id === "issue_source")).toBe(true);
    expect(projectMenu({ ...read, is_home: true }, desktop).some((item) => item.id === "issue_source")).toBe(false);
    expect(projectMenu(workspace({ checkouts: [primary()] }), desktop).some((item) => item.id === "issue_source")).toBe(false);
  });

  it("checks the stored Issue source on a plain folder's one row too, not Automatic (B7)", () => {
    const folderCheckout = primary();
    const source = { kind: "local", label: "Local", name: null, reading: false, failure: null, last_read_at_unix_ms: null };
    const folder = workspace({ is_git: false, checkouts: [folderCheckout], tasks: { source, tasks: [] } as unknown as Workspace["tasks"] });
    const checked = (stored?: string) => folderMenu(folder, folderCheckout, desktop, null, stored).find((item) => item.id === "issue_source")?.choices?.filter((choice) => choice.checked).map((choice) => choice.id);
    expect(checked()).toEqual(["issue_source_auto"]);
    expect(checked("local")).toEqual(["issue_source_local"]);
  });

  it("offers Pin and Remove on a row Herdr shows without a registration (B3, D-14)", () => {
    const unregistered = projectMenu(workspace({ registered: false, checkouts: [primary()] }), desktop);
    expect(unregistered.filter((item) => item.id === "pin" || item.id === "remove_project").map((item) => [item.label, item.unavailable])).toEqual([
      ["Pin", null],
      ["Remove project…", null],
    ]);
  });

  it("opens the new tab in the checkout the home glyph marks", () => {
    const menu = (project: Workspace) => projectMenu(project, desktop).find((item) => item.id === "new_tab_primary")!;
    expect(menu(workspace({ checkouts: [checkout(), primary()] })).unavailable).toBeNull();
    expect(menu(workspace({ checkouts: [checkout()] })).unavailable).toMatch(/no default checkout/);
    expect(menu(workspace({ checkouts: [primary({ exists: false })] })).unavailable).toMatch(/missing/);
  });

  it("draws a checkout's menu in the board's order with the pull request after New tab (B4)", () => {
    const pr = { number: 180, title: "Sidebar readability", url: "https://example.invalid/pull/180", badge: "open" as const, review: null, is_draft: false, state: "pending" as const };
    expect(drawn(checkoutMenu(workspace(), checkout({ pull_request: pr }), desktop))).toEqual([
      "Open",
      "New tab here ⌘T",
      "New group…",
      "Open pull request #180",
      "─",
      "Set purpose…",
      "Set as default checkout",
      "Copy branch name",
      "Copy path",
      "Reveal in Finder",
      "─",
      "Delete worktree…",
    ]);
    // No pull request, no file manager, and a checkout that is not a linked worktree.
    expect(drawn(checkoutMenu(workspace(), primary({ is_primary: false }), browser))).toEqual([
      "Open",
      "New tab here ⌥T",
      "New group…",
      "─",
      "Set purpose…",
      "Set as default checkout",
      "Copy branch name",
      "Copy path",
    ]);
    expect(checkoutMenu(workspace(), checkout(), desktop).find((item) => item.id === "delete_worktree")?.destructive).toBe(true);
    // A pull request GitHub cannot vouch for now is not offered (checkout-pr-glyph-card B10).
    const unavailable = { failure_category: "not_logged_in" as const, available: false, loading: false, stale: false, last_success_at_unix_ms: null, unavailable_reason: "gh is not signed in" };
    expect(checkoutMenu(workspace(), checkout({ pull_request: pr, github: unavailable }), desktop).some((item) => item.id === "open_pull_request")).toBe(false);
  });

  it("says why a checkout cannot become the default (B6, D-09)", () => {
    const reason = (project: Workspace, row: Checkout) => checkoutMenu(project, row, desktop).find((item) => item.id === "set_primary")!.unavailable;
    expect(reason(workspace(), checkout())).toBeNull();
    expect(reason(workspace(), primary())).toBe("Already the default checkout.");
    expect(reason(workspace({ is_git: false }), checkout())).toMatch(/plain folder/);
    expect(reason(workspace({ device_id: "studio", remote_target_id: "studio" }), checkout())).toMatch(/another device/);
    expect(reason(workspace({ registered: false }), checkout())).toMatch(/Pin the project first/);
    expect(reason(workspace(), checkout({ exists: false }))).toMatch(/missing/);
  });

  it("greys out the file manager and the default on a device's checkout and keeps the rest, deletion included (B9)", () => {
    const remote = checkoutMenu(workspace({ device_id: "studio", remote_target_id: "studio" }), checkout(), desktop);
    expect(remote.filter((item) => item.unavailable !== null).map((item) => [item.id, item.unavailable])).toEqual([
      ["set_primary", "Not available for a checkout on another device."],
      ["reveal_external", "Only for files and folders on this computer."],
    ]);
    const project = projectMenu(workspace({ agent_scope: emptyScope(), device_id: "studio", remote_target_id: "studio", checkouts: [primary()] }), desktop);
    expect(project.filter((item) => item.unavailable !== null).map((item) => item.id)).toEqual(["reveal_external"]);
  });

  it("never disables Delete worktree, dirty or unread, and keeps a detached checkout's missing branch", () => {
    const dirty = checkout();
    if (dirty.worktree) {
      dirty.worktree.dirty = true;
      dirty.worktree.deletion_gate.discard = { changed_files: 3, nested: false, status_unknown: false, ignored_repositories: [] };
    }
    const deletion = (row: Checkout) => checkoutMenu(workspace(), row, desktop).find((item) => item.id === "delete_worktree");
    expect(deletion(dirty)?.unavailable).toBeNull();
    expect(deletion(checkout({ worktree: null }))?.unavailable).toBeNull();
    expect(deletion(checkout({ is_worktree: false }))).toBeUndefined();
    expect(checkoutMenu(workspace(), checkout({ branch: null }), desktop).find((item) => item.id === "copy_branch")?.unavailable).toMatch(/Detached/);
  });

  it("names the agents a deletion stops once, by name and state, beside the gate's warnings", () => {
    const pane = (id: string, identity: string | null, status: AgentStatusCode) => ({
      id,
      herdr_label: null,
      terminal_title: null,
      cwd: "/Users/example/hide.worktrees/feature",
      status_code: status,
      requires_close_confirmation: false,
      requires_close_status_check: false,
      identity_label: identity,
    });
    const row = checkout({
      tabs: [
        { id: "t1", workspace_id: "w1", checkout_id: "c1", label: "1", empty: false, delegated: false, panes: [pane("p1", "Fix the parser", "working"), pane("p2", null, "unknown")] },
        { id: "t2", workspace_id: "w1", checkout_id: "c1", label: "2", empty: false, delegated: false, panes: [pane("p3", "Review tests", "idle")] },
      ],
    });
    expect(factsLine(deletionFacts(row, 3))).toBe("Folder removed for good · 3 panes close · stops 2 agents: Fix the parser (Working), Review tests (Idle)");
  });

  it("puts a folder's own checkout items after its project items, past a separator", () => {
    const folder = primary({ is_primary: false, branch: null });
    const menu = folderMenu(workspace({ is_git: false, checkouts: [folder] }), folder, desktop);
    expect(drawn(menu)).toEqual(["Project settings", "New worktree…", "New tab in main ⌘T", "─", "Reveal in Finder", "Copy path", "─", "Pin", "Remove project…", "─", "Open", "New group…", "Set purpose…"]);
    const pr = { number: 7, title: "", url: "https://example.invalid/pull/7", badge: "open" as const, review: null, is_draft: false, state: "pending" as const };
    expect(drawn(folderMenu(workspace({ checkouts: [folder] }), { ...folder, pull_request: pr }, browser)).slice(-5)).toEqual(["─", "Open", "New group…", "Open pull request #7", "Set purpose…"]);
  });

  it("says what removing a project closes and that an unregistered row leaves with Herdr's workspace (B3)", () => {
    const counted = { removal: { pane_count: 2, running_agent_count: 1 } };
    expect(factsLine(projectRemovalFacts(workspace(counted)))).toBe("2 panes close · 1 agent stops · registration only · files stay on disk");
    expect(factsLine(projectRemovalFacts(workspace({ ...counted, registered: false })))).toBe("2 panes close · 1 agent stops · the row goes with them · files stay on disk");
    expect(factsLine(projectRemovalFacts(workspace({ removal: { pane_count: 0, running_agent_count: 0 } })))).toBe("Registration only · files stay on disk");
  });

  it("draws an agent's menu with its ⌥n and without Mark as seen or Stop agent (B7, B8)", () => {
    const agent = legacyAgentRow({ id: "a", pane_id: "p1", identity_label: "배포 전 확인", agent_kind: "claude", mark: "going", group: "working", status_code: "working" as const, changed_at_unix_ms: null, emphasized: false, unread: false, session_id: "0b5e-session" });
    expect(drawn(agentMenu(agent, "⌥3"))).toEqual(["Show ⌥3", "─", "Copy name and ID", "Copy session id", "─", "Close tab…"]);
    expect(drawn(agentMenu(agent, ""))[0]).toBe("Show");
    expect(agentMenu({ ...agent, session_id: null }, "").find((item) => item.id === "copy_session_id")?.unavailable).toMatch(/no session id/);
  });
});

describe("receipts", () => {
  it("reads only the task this page asked for", () => {
    const request = { kind: "worktree_create", afterId: 4, repositoryRoot: "/Users/example/hide", branch: "feature" };
    expect(taskFor(task({}), request, "local")?.id).toBe(5);
    expect(taskFor(task({ id: 4 }), request, "local")).toBeNull();
    expect(taskFor(task({ branch: "other" }), request, "local")).toBeNull();
    expect(taskFor(task({ kind: "checkout_purpose" }), request, "local")).toBeNull();
    expect(taskFor(task({}), null, "local")).toBeNull();
    // The same repository path on another device is another task.
    expect(taskFor(task({}), { ...request, deviceId: "local" }, "local")?.id).toBe(5);
    expect(taskFor(task({ device_id: "studio" }), { ...request, deviceId: "local" }, "local")).toBeNull();
    expect(taskFor(task({ device_id: "studio" }), { ...request, deviceId: "studio" }, "local")?.id).toBe(5);
  });

  it("reads only the removal of the checkout this page asked to delete", () => {
    const removal = { id: 3, repository_root: "/r", checkout_path: "/r-feature", branch: "feature", delete_branch: false, phase: "removing", notice: null };
    expect(removalFor(removal, "local", "/r-feature", 2, "local")?.id).toBe(3);
    expect(removalFor(removal, "local", "/r-other", 2, "local")).toBeNull();
    expect(removalFor(removal, "local", "/r-feature", 3, "local")).toBeNull();
    // The same path on another device is not this page's removal.
    expect(removalFor(removal, "studio", "/r-feature", 2, "local")).toBeNull();
    expect(removalFor({ ...removal, device_id: "studio" }, "studio", "/r-feature", 2, "local")?.id).toBe(3);
  });

  it("marks a checkout as being deleted while checking, closing panes or running Git", () => {
    const removal = { id: 3, repository_root: "/r", checkout_path: "/r-feature", branch: "feature", delete_branch: false, phase: "closing", notice: null };
    expect(checkoutRemoving(removal, "local", "/r-feature", "local")).toBe(true);
    expect(checkoutRemoving({ ...removal, phase: "checking" }, "local", "/r-feature", "local")).toBe(true);
    expect(checkoutRemoving({ ...removal, phase: "removing" }, "local", "/r-feature", "local")).toBe(true);
    // A finished removal took the row away; a failed one gives it back as it was.
    expect(checkoutRemoving({ ...removal, phase: "finished" }, "local", "/r-feature", "local")).toBe(false);
    expect(checkoutRemoving({ ...removal, phase: "failed" }, "local", "/r-feature", "local")).toBe(false);
    expect(checkoutRemoving(removal, "local", "/r-other", "local")).toBe(false);
    expect(checkoutRemoving(removal, "studio", "/r-feature", "local")).toBe(false);
    expect(checkoutRemoving(null, "local", "/r-feature", "local")).toBe(false);
  });
});

it("requires a new Discard choice when the measured repository names change", () => {
  const target = checkout();
  const selected = discardConfirmationKey(target);
  const same = checkout({ worktree: { ...target.worktree!, ignored_repositories: [] } });
  expect(discardConfirmationKey(same)).toBe(selected);
  const changed = checkout({ worktree: { ...target.worktree!, ignored_repositories: ["target/vendor/alpha", "node_modules/beta"] } });
  expect(discardConfirmationKey(changed)).not.toBe(selected);
});

describe("language-dependent sentences", () => {
  const counted = { removal: { pane_count: 2, running_agent_count: 1 } } as Partial<Workspace>;
  it("joins a removal's facts in Korean from the plural-free Korean forms", () => {
    expect(factsLine(manage.projectRemovalFacts(workspace(counted), ko))).toBe("터미널 2개 닫힘 · 에이전트 1개 종료 · 등록만 제거 · 디스크의 파일은 유지");
  });

  it("names the version a remote device lacks, or says it is unknown", () => {
    const remote = workspace({ remote_target_id: "mini" });
    expect(remotePurposeProblem(remote, [{ target_id: "mini", herdr_version: "0.9.0" }] as never)).toBe("Set purpose requires Herdr 0.9.1 or newer on that device; 0.9.0 is installed.");
    expect(remotePurposeProblem(remote, [{ target_id: "mini", herdr_version: null }] as never)).toBe("Set purpose requires Herdr 0.9.1 or newer on that device; its version is unavailable.");
    expect(remotePurposeProblem(remote, [{ target_id: "mini", herdr_version: "0.9.1" }] as never)).toBeNull();
    expect(manage.remotePurposeProblem(remote, [{ target_id: "mini", herdr_version: "0.9.0" }] as never, ko)).toBe("용도를 설정하려면 그 기기에 Herdr 0.9.1 이상이 필요해요. 설치된 버전: 0.9.0");
  });
});

describe("purposeScope", () => {
  it("names the device's Herdr as the only store for a device purpose and adds the branch description here", () => {
    expect(purposeScope("MacBook", "feature")).toBe("Kept in Herdr's workspace metadata on MacBook; its Git config is not changed.");
    expect(purposeScope(null, "feature")).toBe("Kept in Herdr's workspace metadata and as the Git description of feature on this machine.");
    expect(purposeScope(null, null)).toBe("Kept in Herdr's workspace metadata on this machine.");
  });
});
