import { projectRows, checkoutPresentation, checkoutCard } from "../test/legacyAgentScope";
import { emptyScope } from "../test/legacyAgentScope";
import { legacyAgentRow } from "../test/legacyAgentRow";
import { describe, expect, it } from "vitest";
import { activeCheckouts, cardSingleValue, checkoutHasSecondLine, checkoutNameParts, checkoutRowExpansion, projectCheckout, projectRowExpansion, relativeActivity, shownPullRequest } from "./projects";
import { initializeInterfaceI18n } from "./i18n/instance";
import type { AgentRow, Checkout, GithubStatus, PullRequest, Workspace } from "./snapshot";

const t = initializeInterfaceI18n("en").t;
const tKo = initializeInterfaceI18n("ko").t;

function workspace(id: string, extra: Partial<Workspace> = {}): Workspace {
  return { agent_scope: emptyScope(),
    id,
    label: id,
    path: `/h/${id}`,
    device_id: "local",
    registered: true,
    temporary: false,
    pinned: false,
    checkouts: [],
    inactive_checkouts: { expanded: false, checkout_ids: [] },
    ...extra,
  };
}

const NO_MARKS = { turn: 0, going: 0, done: 0, unread: 0 };

describe("activeCheckouts", () => {
  it("leaves out the checkouts the core folded, hid as child-only, or put in Cleanup", () => {
    const rows = ["a", "b", "c", "d", "e"].map((id) => ({ id }) as Checkout);
    const draw = workspace("p", {
      checkouts: rows,
      inactive_checkouts: { expanded: false, checkout_ids: ["b"] },
      session_folds: { empty: ["c"], child_only: ["d"], cleanup: ["e"], empty_open: false, cleanup_open: false, open_prs: 0 },
    });
    expect(activeCheckouts(draw).map((row) => row.id)).toEqual(["a"]);
  });
});

describe("projectRows", () => {
  it("draws pinned rows under their header, then the activity list with the device fold", () => {
    const rows = projectRows(
      [workspace("a", { pinned: true }), workspace("b"), workspace("c"), workspace("d")],
      { inactive_projects: [{ device_id: "local", expanded: false, project_ids: ["c", "d"] }] },
      [],
    );
    expect(rows.map((row) => (row.kind === "workspace" ? `${row.kind}:${row.workspace.id}:${row.level}` : row.kind))).toEqual([
      "header",
      "workspace:a:root",
      "header",
      "workspace:b:root",
      "inactive_projects",
    ]);
    const expanded = projectRows(
      [workspace("b"), workspace("c")],
      { inactive_projects: [{ device_id: "local", expanded: true, project_ids: ["c"] }] },
      [],
    );
    expect(expanded.at(-1)).toMatchObject({ kind: "workspace", level: "child" });
  });

  it("folds projects with no agents after the active ones and before Inactive projects (PRD sidebar-titlebar-refresh B1)", () => {
    const folds = {
      no_agent_projects: [{ device_id: "local", expanded: true, project_ids: ["c"] }],
      inactive_projects: [{ device_id: "local", expanded: false, project_ids: ["d"] }],
    };
    const rows = projectRows([workspace("a", { pinned: true }), workspace("b"), workspace("c"), workspace("d")], folds, []);
    expect(rows.map((row) => (row.kind === "workspace" ? `${row.workspace.id}:${row.level}` : row.kind === "header" ? row.section : row.kind === "raised" ? row.kind : `${row.kind}:${row.count}`))).toEqual([
      "pinned",
      "a:root",
      "recent",
      "b:root",
      "no_agents:1",
      "c:child",
      "inactive_projects:1",
    ]);
  });

  it("omits the pinned header when nothing is pinned", () => {
    const rows = projectRows([workspace("b")], {}, []);
    expect(rows[0]).toEqual({ kind: "header", section: "recent", count: 1 });
  });

  it("raises Needs You then Done above Pinned, in the core's order, and keeps them in the tree", () => {
    const withPanes = (id: string, panes: string[], extra: Partial<Workspace> = {}) =>
      workspace(id, { checkouts: [{ id: `${id}-main`, tabs: [{ id: `${id}-t`, panes: panes.map((pane) => ({ id: pane })) }] } as unknown as Checkout], ...extra });
    const agent = (pane: string, group: string, extra: Partial<AgentRow> = {}) => ({ agent: legacyAgentRow({ pane_id: pane, id: pane, group, ...extra }) as AgentRow, device: null });
    const listed = [
      agent("done-1", "done"),
      agent("ask-2", "needs_you", { lineage_collapsed: false }),
      agent("run", "working"),
      agent("ask-1", "needs_you"),
      // A pane no drawn project owns is not raised: it would have no tree to stay in.
      agent("elsewhere", "needs_you"),
    ];
    const rows = projectRows([withPanes("a", ["done-1", "ask-1"], { pinned: true }), withPanes("b", ["ask-2", "run"])], {}, listed);
    const shape = rows.map((row) =>
      row.kind === "raised" ? `${row.group}:${row.agents.map(({ agent }) => agent.pane_id).join(",")}` : row.kind === "header" ? row.section : `${row.kind}:${row.kind === "workspace" ? row.workspace.id : ""}`,
    );
    // Working is the Agents tab's: the Projects list raises Needs You and Done only.
    expect(shape).toEqual(["needs_you:ask-2,ask-1", "done:done-1", "pinned", "workspace:a", "recent", "workspace:b"]);
    // A raised row never unfolds, whatever the tree below has open.
    const needsYou = rows[0] as Extract<(typeof rows)[number], { kind: "raised" }>;
    expect(needsYou.agents.map(({ agent }) => agent.lineage_collapsed !== false)).toEqual([true, true]);
    expect(listed[1]!.agent.lineage_collapsed).toBe(false);
    // An empty group draws no section.
    expect(projectRows([withPanes("b", ["run"])], {}, listed).map((row) => row.kind)).toEqual(["header", "workspace"]);
  });

  it("draws the five latest Needs You and three latest Done, folding the rest until their group is opened", () => {
    const panes = [...Array.from({ length: 7 }, (_, i) => `ask-${i}`), ...Array.from({ length: 4 }, (_, i) => `done-${i}`)];
    const projects = [workspace("a", { checkouts: [{ id: "a-main", tabs: [{ id: "a-t", panes: panes.map((pane) => ({ id: pane })) }] } as unknown as Checkout] })];
    const listed = panes.map((pane) => ({ agent: legacyAgentRow({ pane_id: pane, id: pane, group: pane.startsWith("ask") ? "needs_you" : "done" }) as AgentRow, device: null }));
    const sections = (open: string[]) =>
      projectRows(projects, {}, listed, null, open).flatMap((row) =>
        row.kind === "raised" ? [{ group: row.group, drawn: row.agents.map(({ agent }) => agent.pane_id), more: row.more.map(({ agent }) => agent.pane_id), expanded: row.expanded }] : [],
      );
    expect(sections([])).toEqual([
      { group: "needs_you", drawn: ["ask-0", "ask-1", "ask-2", "ask-3", "ask-4"], more: ["ask-5", "ask-6"], expanded: false },
      { group: "done", drawn: ["done-0", "done-1", "done-2"], more: ["done-3"], expanded: false },
    ]);
    expect(sections(["done"]).map((row) => [row.group, row.expanded])).toEqual([
      ["needs_you", false],
      ["done", true],
    ]);
  });
});

describe("activity", () => {
  it("rounds recency to the coarsest unit that fits", () => {
    const now = 1_000_000_000;
    expect(relativeActivity(null, now, t)).toBeNull();
    expect(relativeActivity(now + 5000, now, t)).toBe("now");
    expect(relativeActivity(now - 30_000, now, t)).toBe("now");
    expect(relativeActivity(now - 5 * 60_000, now, t)).toBe("5m");
    expect(relativeActivity(now - 3 * 3_600_000, now, t)).toBe("3h");
    expect(relativeActivity(now - 49 * 3_600_000, now, t)).toBe("2d");
  });

  it("writes the recency in the interface language", () => {
    const now = 1_000_000_000;
    expect(relativeActivity(now - 30_000, now, tKo)).toBe("방금");
    expect(relativeActivity(now - 5 * 60_000, now, tKo)).toBe("5분");
    expect(relativeActivity(now - 3 * 3_600_000, now, tKo)).toBe("3시간");
    expect(relativeActivity(now - 49 * 3_600_000, now, tKo)).toBe("2일");
  });
});

describe("checkoutPresentation", () => {
  const now = 1_000_000_000_000;
  const github = (extra: Partial<GithubStatus> = {}): GithubStatus =>
    ({ failure_category: null, available: true, loading: false, stale: false, last_success_at_unix_ms: now, ...extra }) as GithubStatus;
  const pr = (extra: Partial<PullRequest> = {}): PullRequest => ({ number: 155, title: "Browser display", url: "", badge: "open", review: null, is_draft: false, state: "pending", ...extra });
  const project = workspace("repo", { path: "/h/repo", is_git: true });
  const checkout = (extra: Partial<Checkout> = {}): Checkout =>
    ({ agent_scope: emptyScope(),
      id: "c",
      workspace_id: "w",
      label: "feature",
      path: "/h/repo.worktrees/feature",
      branch: "feature",
      purpose: null,
      is_worktree: true,
      exists: true,
      has_panes: false,
      worktree: { branch: "feature", head_sha: "abc1234", last_commit_unix_seconds: (now - 2 * 3_600_000) / 1000 },
      pull_request: null,
      github: github(),
      tabs: [],
      active_tab_id: null,
      strip: [],
      next_tab_label: "Tab 1",
      ...extra,
    }) as Checkout;

  it("draws a known pull request's mark before the kind of checkout, and names its state in the tooltip", () => {
    expect(checkoutPresentation(project, checkout({ pull_request: pr() }), now, t)).toMatchObject({ kind: "pull_request", stale: false, settled: false });
    expect(checkoutPresentation(project, checkout({ pull_request: pr({ badge: "merged", state: "merged" }) }), now, t)).toMatchObject({ kind: "pull_request", settled: true });
    expect(checkoutPresentation(project, checkout({ pull_request: pr({ state: "failed" }) }), now, t).detail.split("\n")[0]).toBe("#155 · CI failed · Browser display");
  });

  it("draws a missing folder's kind in danger rather than its pull request's mark", () => {
    expect(checkoutPresentation(project, checkout({ pull_request: pr(), exists: false }), now, t)).toMatchObject({ kind: "branch", kindTone: "text-destructive" });
  });

  it("dims a stale pull request's mark and falls back to the branch when GitHub could not answer", () => {
    expect(checkoutPresentation(project, checkout({ pull_request: pr(), github: github({ stale: true }) }), now, t)).toMatchObject({ kind: "pull_request", stale: true });
    expect(checkoutPresentation(project, checkout({ pull_request: pr(), github: github({ available: false, failure_category: "not_logged_in" }) }), now, t).kind).toBe("branch");
  });

  it("names the primary checkout, a detached one and a plain folder", () => {
    expect(checkoutPresentation(project, checkout({ is_primary: true, is_worktree: false, path: "/h/repo" }), now, t).kind).toBe("primary");
    expect(checkoutPresentation(project, checkout({ worktree: { branch: null, head_sha: "abc1234" } as Checkout["worktree"] }), now, t).kind).toBe("detached");
    expect(checkoutPresentation(workspace("notes", { is_git: false }), checkout({ worktree: null, is_worktree: false }), now, t).kind).toBe("folder");
  });

  it("uses the snapshot home choice even on a linked worktree with a pull request", () => {
    expect(checkoutPresentation(project, checkout({ is_primary: true, pull_request: pr() }), now, t).kind).toBe("primary");
    expect(checkoutPresentation(project, checkout({ is_primary: false, is_worktree: false, path: project.path }), now, t).kind).toBe("branch");
    expect(checkoutPresentation({ ...project, device_id: "remote" }, checkout({ is_primary: true }), now, t).kind).toBe("primary");
  });

  it("dates the last commit, and draws a missing folder in danger with no age", () => {
    expect(checkoutPresentation(project, checkout(), now, t).age).toBe("2h");
    expect(checkoutPresentation(project, checkout({ exists: false }), now, t)).toMatchObject({ kindTone: "text-destructive", age: null });
  });

  it("holds line two back until Git has been read", () => {
    expect(checkoutPresentation(project, checkout({ worktree: null }), now, t)).toMatchObject({ secondLineReady: false, age: null });
  });

  it("lists the agents by state in the tooltip", () => {
    const view = checkoutPresentation(project, checkout({ agent_summary: { representative_pane_id: "p1", needs_you: 1, done: 0, working: 2, seen: 1, unknown: 1, marks: NO_MARKS } }), now, t);
    expect(view.detail.split("\n")).toEqual(["Needs You: 1 · Working: 2 · Seen: 1 (1 Unknown)", "feature", "/h/repo.worktrees/feature"]);
  });
});

describe("checkoutCard", () => {
  const now = 1_000_000_000_000;
  const project = workspace("repo", { path: "/h/repo", is_git: true });
  const pr = (extra: Partial<PullRequest> = {}): PullRequest => ({ number: 180, title: "Sidebar readability", url: "https://example.invalid/pull/180", badge: "review", review: "approved", is_draft: false, checks: "passing", state: "mergeable", ...extra });
  const summary = (marks: Partial<typeof NO_MARKS>, counts: Partial<{ needs_you: number; done: number; working: number; seen: number }> = {}) => ({
    representative_pane_id: null,
    needs_you: 0,
    done: 0,
    working: 0,
    seen: 0,
    unknown: 0,
    ...counts,
    marks: { ...NO_MARKS, ...marks },
  });
  const checkout = (extra: Partial<Checkout> = {}): Checkout =>
    ({ agent_scope: emptyScope(),
      id: "c",
      workspace_id: "w",
      label: "feature",
      path: "/h/repo.worktrees/feature",
      branch: "feature",
      purpose: null,
      is_worktree: true,
      exists: true,
      has_panes: false,
      worktree: { branch: "feature", head_sha: "abc1234def", last_commit_unix_seconds: (now - 2 * 3_600_000) / 1000 },
      pull_request: null,
      tabs: [],
      active_tab_id: null,
      strip: [],
      next_tab_label: "Tab 1",
      ...extra,
    }) as Checkout;
  const keys = (card: ReturnType<typeof checkoutCard>) => card.rows.map((row) => row.key);

  it("heads a pull request's card with its badge, number, url and title, then every row with a value (B5)", () => {
    const card = checkoutCard(project, checkout({ pull_request: pr(), agent_summary: summary({ turn: 1, going: 2 }, { needs_you: 1, working: 2 }) }), now, t);
    expect(card.header).toEqual({
      kind: "pull_request",
      badge: { label: "Approved", draft: false },
      state: "mergeable",
      number: 180,
      url: "https://example.invalid/pull/180",
      title: "Sidebar readability",
    });
    expect(card.rows).toEqual([
      { key: "review", label: "Review", value: "Approved", tone: "text-pr-mergeable" },
      { key: "checks", label: "Checks", value: "Passing", tone: "text-pr-mergeable" },
      { key: "branch", label: "Branch", value: "feature" },
      { key: "agents", label: "Agents", marks: { ...NO_MARKS, turn: 1, going: 2 } },
      { key: "commit", label: "Commit", value: "2h ago" },
      { key: "path", label: "Path", value: "/h/repo.worktrees/feature" },
    ]);
  });

  it("colors Checks by result and leaves the row out when there are none or they are unknown (B6)", () => {
    expect(checkoutCard(project, checkout({ pull_request: pr({ checks: "failed" }) }), now, t).rows[1]).toEqual({ key: "checks", label: "Checks", value: "Failed", tone: "text-pr-failed" });
    expect(checkoutCard(project, checkout({ pull_request: pr({ checks: "pending" }) }), now, t).rows[1]).toMatchObject({ value: "Pending", tone: "text-pr-pending" });
    expect(keys(checkoutCard(project, checkout({ pull_request: pr({ checks: "none" }) }), now, t))).toEqual(["review", "branch", "commit", "path"]);
    expect(keys(checkoutCard(project, checkout({ pull_request: pr({ checks: "unknown" }) }), now, t))).toEqual(["review", "branch", "commit", "path"]);
    expect(keys(checkoutCard(project, checkout({ pull_request: pr({ checks: undefined }) }), now, t))).toEqual(["review", "branch", "commit", "path"]);
  });

  it("leaves Review out of an open pull request with no decision, and names the others (B6)", () => {
    expect(keys(checkoutCard(project, checkout({ pull_request: pr({ badge: "open", review: null }) }), now, t))).toEqual(["checks", "branch", "commit", "path"]);
    expect(checkoutCard(project, checkout({ pull_request: pr({ review: "changes_requested" }) }), now, t).rows[0]).toMatchObject({ value: "Changes requested", tone: "text-pr-failed" });
    expect(checkoutCard(project, checkout({ pull_request: pr({ review: "review_required" }) }), now, t).rows[0]).toMatchObject({ value: "Review required", tone: "text-muted-foreground" });
  });

  it("has no header without a pull request and draws only the rows with a value (B8, D-09)", () => {
    const plain = checkoutCard(project, checkout(), now, t);
    expect(plain.header).toBeNull();
    expect(keys(plain)).toEqual(["branch", "commit", "path"]);
    // Git not read yet: neither branch nor commit, no loading row.
    expect(keys(checkoutCard(project, checkout({ worktree: null }), now, t))).toEqual(["path"]);
    // No agents: no Agents row; agents with a zero sum: none either.
    expect(keys(checkoutCard(project, checkout({ agent_summary: summary({}) }), now, t))).toEqual(["branch", "commit", "path"]);
    // A commit whose age is not known has no row.
    expect(keys(checkoutCard(project, checkout({ worktree: { branch: "feature", head_sha: "abc1234def" } as Checkout["worktree"] }), now, t))).toEqual(["branch", "path"]);
  });

  it("names a detached HEAD by its short sha, dates the first minute as now, and a folder has only its path and agents", () => {
    expect(checkoutCard(project, checkout({ worktree: { branch: null, head_sha: "abc1234def", last_commit_unix_seconds: now / 1000 } as Checkout["worktree"] }), now, t).rows).toEqual([
      { key: "branch", label: "Branch", value: "Detached HEAD at abc1234" },
      { key: "commit", label: "Commit", value: "now" },
      { key: "path", label: "Path", value: "/h/repo.worktrees/feature" },
    ]);
    const folder = workspace("notes", { is_git: false });
    expect(keys(checkoutCard(folder, checkout({ worktree: null, is_worktree: false, path: "/h/notes", agent_summary: summary({ going: 1 }, { working: 1 }) }), now, t))).toEqual(["agents", "path"]);
  });

  it("words the detached HEAD row and the commit age in the interface language", () => {
    const card = checkoutCard(project, checkout({ worktree: { branch: null, head_sha: "abc1234def", last_commit_unix_seconds: (now - 3 * 3_600_000) / 1000 } as Checkout["worktree"] }), now, tKo);
    expect(card.rows).toEqual([
      { key: "branch", label: "브랜치", value: "abc1234에서 분리된 HEAD" },
      { key: "commit", label: "커밋", value: "3시간 전" },
      { key: "path", label: "경로", value: "/h/repo.worktrees/feature" },
    ]);
  });

  it("heads a missing folder with Folder missing over its path alone, whatever else is known", () => {
    const card = checkoutCard(project, checkout({ exists: false, pull_request: pr(), agent_summary: summary({}, { seen: 1 }) }), now, t);
    expect(card.header).toEqual({ kind: "missing", label: "Folder missing" });
    expect(card.rows).toEqual([{ key: "path", label: "Path", value: "/h/repo.worktrees/feature" }]);
  });

  it("falls back to a text tooltip only when the card would hold one value and no header (D-09)", () => {
    const folder = workspace("notes", { is_git: false });
    expect(cardSingleValue(checkoutCard(folder, checkout({ worktree: null, is_worktree: false, path: "/h/notes" }), now, t))).toBe("/h/notes");
    expect(cardSingleValue(checkoutCard(project, checkout(), now, t))).toBeNull();
    expect(cardSingleValue(checkoutCard(project, checkout({ exists: false }), now, t))).toBeNull();
  });

  it("shows the pull request a stale refresh kept and none an unavailable lookup lost", () => {
    const github = (extra: Partial<GithubStatus> = {}): GithubStatus =>
      ({ failure_category: null, available: true, loading: false, stale: false, last_success_at_unix_ms: now, ...extra }) as GithubStatus;
    expect(shownPullRequest(checkout({ pull_request: pr(), github: github({ stale: true, available: false }) }))?.number).toBe(180);
    expect(shownPullRequest(checkout({ pull_request: pr(), github: github({ available: false, failure_category: "not_logged_in" }) }))).toBeNull();
    expect(shownPullRequest(checkout({ pull_request: null }))).toBeNull();
    expect(checkoutCard(project, checkout({ pull_request: pr(), github: github({ available: false, failure_category: "not_logged_in" }) }), now, t).header).toBeNull();
    // The card words the category; gh's own words stay in the core's log.
    expect(checkoutPresentation(project, checkout({ github: github({ stale: true, failure_category: "network_or_rate_limit" }) }), now, t).detail).toContain("GitHub didn't answer");
    expect(checkoutPresentation(project, checkout({ github: github({ stale: true, failure_category: "from_a_newer_core" as GithubStatus["failure_category"] }) }), now, t).detail).toContain("GitHub couldn't be read");
  });
});

describe("sidebar row activation", () => {
  it("opens folded rows and preserves an expanded row when returning from another scope", () => {
    expect(checkoutRowExpansion(true, false, false)).toBe(true);
    expect(checkoutRowExpansion(true, true, false)).toBe(true);
    expect(checkoutRowExpansion(true, false, true)).toBe(true);
    expect(checkoutRowExpansion(true, true, true)).toBe(false);
    expect(checkoutRowExpansion(false, true, true)).toBeUndefined();
  });

  it("unfolds a project unless its return checkout is already in front", () => {
    const target = { id: "main" } as Checkout;
    expect(projectRowExpansion(workspace("repo", { expanded: false }), target, "main", true)).toBe(true);
    expect(projectRowExpansion(workspace("repo", { expanded: true }), target, "other", true)).toBe(true);
    expect(projectRowExpansion(workspace("repo", { expanded: true }), target, "main", true)).toBe(false);
    expect(projectRowExpansion(workspace("repo", { expanded: true }), target, "main", false)).toBeUndefined();
  });

  it("returns to the latest usable checkout on the same device before primary and row order", () => {
    const checkouts = [
      { id: "first", exists: true }, { id: "primary", exists: true, is_primary: true },
      { id: "recent", exists: true }, { id: "missing", exists: false },
    ] as Checkout[];
    const project = workspace("repo", { agent_scope: emptyScope(), device_id: "mini", checkouts });
    const visit = (device_id: string, checkout_id: string) => ({ device_id, checkout_id, project_name: "repo", branch: checkout_id });
    expect(projectCheckout(project, [visit("local", "first"), visit("mini", "missing"), visit("mini", "recent")])?.id).toBe("recent");
    expect(projectCheckout(project)?.id).toBe("primary");
    expect(projectCheckout({ ...project, checkouts: [checkouts[0]!, checkouts[2]!] })?.id).toBe("first");
    expect(projectCheckout({ ...project, checkouts: [checkouts[3]!] })).toBeNull();
  });
});

describe("checkout row lines (PRD sidebar-typography D-04, D-06)", () => {
  const project = workspace("p", { is_git: true });
  const now = Date.UTC(2026, 8, 27);
  const read = (extra: Partial<Checkout> = {}) =>
    checkoutPresentation(
      project,
      { agent_scope: emptyScope(),
        id: "c",
        workspace_id: "w",
        label: "feature",
        path: "/h/p/feature",
        branch: "feature",
        purpose: null,
        is_worktree: true,
        exists: true,
        has_panes: false,
        worktree: { branch: "feature", head_sha: "abc1234", last_commit_unix_seconds: now / 1000 - 3600 },
        pull_request: null,
        tabs: [],
        active_tab_id: null,
        strip: [],
        next_tab_label: "Tab 1",
        ...extra,
      } as Checkout,
      now, t
    );

  it("draws line two only for a purpose or a raised-from parent, agents or not", () => {
    const withAgents = read({ agent_summary: { representative_pane_id: null, needs_you: 0, done: 0, working: 2, seen: 0, unknown: 0, marks: { ...NO_MARKS, going: 2 } } });
    expect(checkoutHasSecondLine(withAgents, null, null)).toBe(false);
    expect(checkoutHasSecondLine(withAgents, "사이드바 가독성 개선", null)).toBe(true);
    expect(checkoutHasSecondLine(withAgents, null, "메인 체크아웃 정리에서")).toBe(true);
    expect(checkoutHasSecondLine(read(), "purpose", null)).toBe(true);
    expect(checkoutHasSecondLine(read({ worktree: null }), "purpose", null)).toBe(false);
  });

  it("mutes a name's path prefix up to its first slash", () => {
    expect(checkoutNameParts("prd/sidebar-typography")).toEqual({ prefix: "prd/", rest: "sidebar-typography" });
    expect(checkoutNameParts("gen-prd/a/b")).toEqual({ prefix: "gen-prd/", rest: "a/b" });
    expect(checkoutNameParts("main")).toEqual({ prefix: "", rest: "main" });
    expect(checkoutNameParts("/abs")).toEqual({ prefix: "", rest: "/abs" });
    expect(checkoutNameParts("trailing/")).toEqual({ prefix: "", rest: "trailing/" });
  });
});

