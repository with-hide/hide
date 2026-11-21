import { emptyScope, legacyRest } from "../test/legacyAgentScope";
import { legacyAgentRow } from "../test/legacyAgentRow";
// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterAll, afterEach, expect, it, vi } from "vitest";
import tokensText from "../../design/tokens.json?raw";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { Sidebar } from "./sidebar";
import type { AgentRow, GithubStatus, PrState, PullRequest, SessionPull, SnapshotRest, Workspace } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import type { DispatchFn } from "./ws";

// PRD project-tree-pr-family: a 프로젝트 root row's PR mark draws the core's
// list of its own and every descendant's PRs as its 태스크 row does (B1, B3,
// B4, B9, B10), a child row keeps its own (B5), and several list each PR with
// who holds it (B7). Expected marks come from the PRD and docs/UI_BEHAVIOR.md.

const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => browserCanvas.restore());

const LOCAL = { id: "local", label: "This Mac", kind: "local", state: "local", message: null };

/** One PR of a row's list; a descendant's names who holds it. */
type Pull = { repo: string; number: number; state: PrState; holders?: string[]; stale?: boolean };

/** A working agent whose row carries `pulls`, the list the core lays on it, worst first as the core orders it. */
function agent(pane: string, label: string, pulls: Pull[], lineage: Partial<AgentRow> = {}): AgentRow {
  const seed = legacyAgentRow({
    id: `agent:${pane}`, pane_id: pane, identity_label: label, agent_kind: "claude", mark: "going", group: "working",
    status_code: "working", changed_at_unix_ms: null, emphasized: false, unread: false, demand: "none", activity: "working", ...lineage,
  });
  const listed: SessionPull[] = pulls.map(({ repo, number, state, holders, stale }) => ({
    url: `https://github.com/acme/${repo}/pull/${number}`, number, title: `${repo} ${number}`, state,
    own: !holders || holders.includes(label), holders: holders ?? [label], stale: stale ?? false, last_read_at_unix_ms: stale ? READ.last_success_at_unix_ms : null,
  }));
  return { ...seed, pulls: listed.length ? { pulls: listed } : null } as AgentRow;
}

/** What the project's GitHub read answered: the PRs it lists and how the read stands. */
type Github = { pulls: PullRequest[]; status: GithubStatus };

/** One project whose main checkout runs `agents` in one tab; `drawn` is the core's opened tree, root first with depths, else roots only. */
function catalog(agents: AgentRow[], drawn?: [string, number][], github?: Github): SnapshotRest {
  const project = { agent_scope: emptyScope(),
    id: "workspace:alpha", label: "alpha", path: "/fixture/alpha", device_id: "local", is_git: true, registered: true, temporary: false, pinned: false,
    pull_requests: github?.pulls,
    inactive_checkouts: { expanded: false, checkout_ids: [] },
    checkouts: [{ agent_scope: emptyScope(),
      id: "checkout:alpha-main", workspace_id: "workspace:alpha", label: "main", path: "/fixture/alpha", branch: "main",
      is_primary: true, is_worktree: false, exists: true, active_tab_id: "tab:alpha", strip: [],
      worktree: { branch: "main", head_sha: null, last_commit_unix_seconds: null },
      github: github?.status,
      tabs: [{ id: "tab:alpha", label: "main", panes: agents.map((row) => ({ id: row.pane_id })) }],
    }],
  } as unknown as Workspace;
  const navigator = { siblings_shown: 5, devices: [LOCAL], workspaces: [project], agents, focused_device_id: "local", focused_checkout_id: "checkout:alpha-main" };
  const rest = legacyRest({ navigator } as unknown as SnapshotRest, agents);
  if (drawn) {
    const tree = rest.navigator!.workspaces![0]!.checkouts[0]!.agent_scope.sidebar_tree;
    tree.rows = tree.visible_rows = drawn.map(([pane_id, depth]) => ({ pane_id, occurrence: 0, depth }));
  }
  return rest;
}

let cleanup: (() => Promise<void>) | null = null;
/** What the rendered sidebar sent the core, in order. */
let events: Parameters<DispatchFn>[0][] = [];
const sent = (kind: string) => events.filter((event) => event.kind === kind).map((event) => event.payload);
afterEach(async () => {
  await cleanup?.();
  cleanup = null;
});

async function render(agents: AgentRow[], drawn?: [string, number][], github?: Github) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  // jsdom loads no stylesheet, and the sidebar reads its width tokens from the document: take them from the token source.
  const tokens = (JSON.parse(tokensText) as { tokens: Record<string, { value: number }> }).tokens;
  const widthTokens = ["--size-sidebar-ideal", "--size-sidebar-min", "--size-sidebar-max"];
  for (const name of widthTokens) document.documentElement.style.setProperty(name, `${tokens[name]?.value}px`);
  const shell = useShellStore.getState();
  const ui = useUiStore.getState();
  useUiStore.setState({ screen: { kind: "workspace" }, overviewOpen: false });
  useShellStore.setState({ rest: catalog(agents, drawn, github), agents, connection: "live" });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  events = [];
  const actions = createActions((event) => { events.push(event); return true; });
  await act(async () => root.render(<TooltipProvider><Sidebar actions={actions} /></TooltipProvider>));
  cleanup = async () => {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(shell, true);
    useUiStore.setState(ui, true);
    for (const name of widthTokens) document.documentElement.style.removeProperty(name);
    vi.unstubAllGlobals();
  };
  return container;
}

const mark = (container: HTMLElement, pane: string) => container.querySelector<HTMLElement>(`[data-pane="${pane}"] [data-agent-pr]`);
const counts = (element: HTMLElement | null) => Array.from(element?.querySelectorAll("[data-pr-count]") ?? [], (part) => [part.getAttribute("data-pr-count"), part.textContent]);

it("draws a root's descendants' PR on its row when the root made none, and nothing when no one did", async () => {
  const container = await render([
    agent("pane:root", "릴리스 스킬과 로컬 검증 체계", [{ repo: "app", number: 1073, state: "pending", holders: ["검증 스크립트"] }], { lineage_child_pane_ids: ["pane:child"] }),
    agent("pane:child", "검증 스크립트", [{ repo: "app", number: 1073, state: "pending" }], { delegated: true, lineage_parent_pane_id: "pane:root" }),
  ]);
  expect(mark(container, "pane:root")?.textContent).toBe("#" + "1073");
  expect(mark(container, "pane:root")?.getAttribute("aria-label")).toBe("#" + "1073 CI running or review pending");
  await cleanup?.();

  const none = await render([
    agent("pane:root", "릴리스 스킬과 로컬 검증 체계", [], { lineage_child_pane_ids: ["pane:child"] }),
    agent("pane:child", "검증 스크립트", [], { delegated: true, lineage_parent_pane_id: "pane:root" }),
  ]);
  expect(none.querySelector("[data-agent-pr]")).toBeNull();
});

it("counts a PR held by the root and a descendant once, and the same number in another repository apart", async () => {
  const container = await render([
    // Opened this time; the first test's root is folded, and the mark is the same either way (B4).
    agent("pane:root", "루트", [{ repo: "app", number: 7, state: "mergeable", holders: ["루트", "자식"] }, { repo: "site", number: 7, state: "merged", holders: ["손자"] }], { lineage_child_pane_ids: ["pane:child"], lineage_collapsed: false }),
    agent("pane:child", "자식", [{ repo: "app", number: 7, state: "mergeable" }, { repo: "site", number: 7, state: "merged", holders: ["손자"] }], { delegated: true, lineage_parent_pane_id: "pane:root", lineage_child_pane_ids: ["pane:grandchild"], lineage_collapsed: false }),
    agent("pane:grandchild", "손자", [{ repo: "site", number: 7, state: "merged" }], { delegated: true, lineage_parent_pane_id: "pane:child" }),
  ], [["pane:root", 0], ["pane:child", 1], ["pane:grandchild", 2]]);
  const root = mark(container, "pane:root");
  expect(counts(root)).toEqual([["mergeable", "1"], ["merged", "1"]]);
  expect(root?.getAttribute("aria-label")).toBe("PR 2 · Mergeable 1 · Merged 1");
  // A child row keeps its own PR, as the 태스크 child row does (B5).
  const child = mark(container, "pane:child");
  expect(child?.textContent).toBe("#7");
  expect(mark(container, "pane:grandchild")?.getAttribute("aria-label")).toBe("#7 Merged");

  // Under the pointer the root's list names each PR worst first with the sessions that hold it (B7).
  const trigger = container.querySelector<HTMLElement>('[data-pr-hover="pane:root"]')!;
  await act(async () => { trigger.dispatchEvent(new FocusEvent("focusin", { bubbles: true })); });
  const list = document.querySelector<HTMLElement>('[data-pr-list="pane:root"]');
  expect(Array.from(list?.querySelectorAll("[data-pr-list-item]") ?? [], (item) => item.textContent)).toEqual(["#7app 7루트 · 자식", "#7site 7손자"]);
});

it("draws the ask instead of a PR mark on a raised root row, whatever its descendants hold", async () => {
  const container = await render([
    agent("pane:root", "루트", [{ repo: "app", number: 9, state: "pending", holders: ["자식"] }], { lineage_child_pane_ids: ["pane:child"], group: "needs_you", demand: "question", mark: "turn" }),
    agent("pane:child", "자식", [{ repo: "app", number: 9, state: "pending" }], { delegated: true, lineage_parent_pane_id: "pane:root" }),
  ]);
  const raised = container.querySelector('[data-raised-group="needs_you"] [data-pane="pane:root"]');
  expect(raised).not.toBeNull();
  expect(raised?.querySelector("[data-agent-pr]")).toBeNull();
});

/** A root that made no PR and the child that made PR 1073, as the operator's example has it. */
const delegated = (stale = false) => [
  agent("pane:root", "릴리스 스킬과 로컬 검증 체계", [{ repo: "app", number: 1073, state: "pending", holders: ["검증 스크립트"], stale }], { lineage_child_pane_ids: ["pane:child"] }),
  agent("pane:child", "검증 스크립트", [{ repo: "app", number: 1073, state: "pending", stale }], { delegated: true, lineage_parent_pane_id: "pane:root" }),
];
const PULL: PullRequest = { number: 1073, title: "릴리스 검증 스크립트", url: "https://github.com/acme/app/pull/1073", badge: "open", review: null, is_draft: false, checks: "pending", state: "pending", head_branch: "feat/verify" };
const READ: GithubStatus = { failure_category: null, available: true, loading: false, stale: false, last_success_at_unix_ms: Date.UTC(2026, 9, 18, 9, 0) };
const hover = async (container: HTMLElement, pane: string) => {
  const trigger = container.querySelector<HTMLElement>(`[data-pr-hover="${pane}"]`)!;
  await act(async () => { trigger.dispatchEvent(new FocusEvent("focusin", { bubbles: true })); });
};

it("shows the PR card under the pointer when a root's one PR is its child's", async () => {
  const container = await render(delegated(), undefined, { pulls: [PULL], status: READ });
  expect(mark(container, "pane:root")?.getAttribute("data-stale")).toBeNull();
  await hover(container, "pane:root");
  const card = document.querySelector<HTMLElement>('[data-checkout-card="pull_request"]');
  expect(card?.textContent).toContain("릴리스 검증 스크립트");
  expect(document.querySelector('[data-pr-list="pane:root"]')).toBeNull();
});

it("dims a root's gathered mark while GitHub cannot be read and says when it last read", async () => {
  const container = await render(delegated(true), undefined, { pulls: [PULL], status: { ...READ, available: false, stale: true } });
  const root = mark(container, "pane:root");
  expect(root?.getAttribute("data-stale")).toBe("true");
  expect(root?.getAttribute("aria-label")).toMatch(new RegExp("^#" + "1073 CI running or review pending · GitHub last read .+ · cannot read now$"));
  await hover(container, "pane:root");
  expect(document.querySelector('[data-pr-list="pane:root"]')?.textContent).toMatch(/GitHub last read .+ · cannot read now/);
});

// Operator decision 2026-11-09 (docs/UI_BEHAVIOR.md, A card opens where it
// was pressed): a PR pressed on a 프로젝트 row, a parent's or a child's, opens
// its card in the slot of that row's pane as one pr_open, never a page; the
// browser modifier still opens it on GitHub.
const MERGEABLE: PullRequest = { ...PULL, number: 1080, title: "루트 정리", url: "https://github.com/acme/app/pull/1080", state: "mergeable", checks: "passing", head_branch: "feat/tidy" };
const family = () => [
  agent("pane:root", "루트", [{ repo: "app", number: 1073, state: "pending", holders: ["자식"] }, { repo: "app", number: 1080, state: "mergeable", holders: ["루트"] }], { lineage_child_pane_ids: ["pane:child"], lineage_collapsed: false }),
  agent("pane:child", "자식", [{ repo: "app", number: 1073, state: "pending" }], { delegated: true, lineage_parent_pane_id: "pane:root" }),
];

it("opens a child's PR pressed in a parent's 프로젝트 row list in the parent's pane slot", async () => {
  const container = await render(family(), [["pane:root", 0], ["pane:child", 1]], { pulls: [PULL, MERGEABLE], status: READ });
  await hover(container, "pane:root");
  const item = document.querySelector<HTMLElement>('[data-pr-list="pane:root"] [data-pr-list-item="1073"]')!;
  await act(async () => item.click());
  expect(sent("pr_open")).toHaveLength(1);
  expect(sent("pr_open")[0]).toMatchObject({ workspace_id: "workspace:alpha", pr_number: 1073, at: { pane: "pane:root" } });
  expect(sent("browser_open")).toEqual([]);
});

it("opens a child row's own PR from its card in the child's pane slot, and on GitHub with the browser modifier", async () => {
  const open = vi.spyOn(window, "open").mockReturnValue(null);
  const container = await render(family(), [["pane:root", 0], ["pane:child", 1]], { pulls: [PULL, MERGEABLE], status: READ });
  await hover(container, "pane:child");
  const press = document.querySelector<HTMLElement>('[data-checkout-card-open="1073"]')!;
  await act(async () => press.click());
  expect(sent("pr_open")).toHaveLength(1);
  expect(sent("pr_open")[0]).toMatchObject({ workspace_id: "workspace:alpha", pr_number: 1073, at: { pane: "pane:child" } });

  await hover(container, "pane:child");
  const again = document.querySelector<HTMLElement>('[data-checkout-card-open="1073"]')!;
  await act(async () => again.dispatchEvent(new MouseEvent("click", { bubbles: true, metaKey: true, ctrlKey: true })));
  expect(open).toHaveBeenCalledWith(PULL.url, "_blank", "noopener,noreferrer");
  expect(sent("pr_open")).toHaveLength(1);
  expect(sent("browser_open")).toEqual([]);
  open.mockRestore();
});
