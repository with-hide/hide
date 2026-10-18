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
import type { AgentPullRequest, AgentRow, PrState, SnapshotRest, Workspace } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

// PRD project-tree-pr-family: a 프로젝트 root row's PR mark gathers its own
// and every descendant's PRs as its 태스크 row does (B1, B3, B4, B9, B10), a
// child row keeps its own (B5), and several list each PR with who holds it
// (B7). Expected marks come from the PRD and docs/UI_BEHAVIOR.md.

const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => browserCanvas.restore());

const LOCAL = { id: "local", label: "This Mac", kind: "local", state: "local", message: null };

type Pull = { repo: string; number: number; state: PrState };

/** A working agent holding `pulls`, the core's PR summary over them worst first as the core orders it. */
function agent(pane: string, label: string, pulls: Pull[], lineage: Partial<AgentRow> = {}): AgentRow {
  const seed = legacyAgentRow({
    id: `agent:${pane}`, pane_id: pane, identity_label: label, agent_kind: "claude", mark: "going", group: "working",
    status_code: "working", changed_at_unix_ms: null, emphasized: false, unread: false, demand: "none", activity: "working", ...lineage,
  });
  const rows: AgentPullRequest[] = pulls.map(({ repo, number }) => ({ number, title: `${repo} ${number}`, url: `https://github.com/acme/${repo}/pull/${number}`, badge: "open", checks: "pending", head_branch: "feat/work", closing_issues: [], live: true, duty: true, created: true, settled_at_unix_ms: null }));
  return {
    ...seed,
    state: { ...seed.state, pr: pulls.length ? { count: pulls.length, worst: pulls[0]!.state, pulls: pulls.map(({ state }, index) => ({ state, index })) } : null },
    request: { verb: "working", verb_since_unix_ms: 0, request: null, later_by: null, reply: null, pull_requests: rows },
  } as AgentRow;
}

/** One project whose main checkout runs `agents` in one tab; `drawn` is the core's opened tree, root first with depths, else roots only. */
function catalog(agents: AgentRow[], drawn?: [string, number][]): SnapshotRest {
  const project = { agent_scope: emptyScope(),
    id: "workspace:alpha", label: "alpha", path: "/fixture/alpha", device_id: "local", is_git: true, registered: true, temporary: false, pinned: false,
    inactive_checkouts: { expanded: false, checkout_ids: [] },
    checkouts: [{ agent_scope: emptyScope(),
      id: "checkout:alpha-main", workspace_id: "workspace:alpha", label: "main", path: "/fixture/alpha", branch: "main",
      is_primary: true, is_worktree: false, exists: true, active_tab_id: "tab:alpha", strip: [],
      worktree: { branch: "main", head_sha: null, last_commit_unix_seconds: null },
      tabs: [{ id: "tab:alpha", label: "main", panes: agents.map((row) => ({ id: row.pane_id })) }],
    }],
  } as unknown as Workspace;
  const navigator = { devices: [LOCAL], workspaces: [project], agents, focused_device_id: "local", focused_checkout_id: "checkout:alpha-main" };
  const rest = legacyRest({ navigator } as unknown as SnapshotRest, agents);
  if (drawn) {
    const tree = rest.navigator!.workspaces![0]!.checkouts[0]!.agent_scope.sidebar_tree;
    tree.rows = tree.visible_rows = drawn.map(([pane_id, depth]) => ({ pane_id, occurrence: 0, depth }));
  }
  return rest;
}

let cleanup: (() => Promise<void>) | null = null;
afterEach(async () => {
  await cleanup?.();
  cleanup = null;
});

async function render(agents: AgentRow[], drawn?: [string, number][]) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  // jsdom loads no stylesheet, and the sidebar reads its width tokens from the document: take them from the token source.
  const tokens = (JSON.parse(tokensText) as { tokens: Record<string, { value: number }> }).tokens;
  const widthTokens = ["--size-sidebar-ideal", "--size-sidebar-min", "--size-sidebar-max"];
  for (const name of widthTokens) document.documentElement.style.setProperty(name, `${tokens[name]?.value}px`);
  const shell = useShellStore.getState();
  const ui = useUiStore.getState();
  useUiStore.setState({ screen: { kind: "workspace" }, overviewOpen: false });
  useShellStore.setState({ rest: catalog(agents, drawn), agents, connection: "live" });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><Sidebar actions={createActions(() => true)} /></TooltipProvider>));
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
    agent("pane:root", "릴리스 스킬과 로컬 검증 체계", [], { lineage_child_pane_ids: ["pane:child"] }),
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
    agent("pane:root", "루트", [{ repo: "app", number: 7, state: "mergeable" }], { lineage_child_pane_ids: ["pane:child"], lineage_collapsed: false }),
    agent("pane:child", "자식", [{ repo: "app", number: 7, state: "mergeable" }], { delegated: true, lineage_parent_pane_id: "pane:root", lineage_child_pane_ids: ["pane:grandchild"], lineage_collapsed: false }),
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
    agent("pane:root", "루트", [], { lineage_child_pane_ids: ["pane:child"], group: "needs_you", demand: "question", mark: "turn" }),
    agent("pane:child", "자식", [{ repo: "app", number: 9, state: "pending" }], { delegated: true, lineage_parent_pane_id: "pane:root" }),
  ]);
  const raised = container.querySelector('[data-raised-list] [data-pane="pane:root"]');
  expect(raised).not.toBeNull();
  expect(raised?.querySelector("[data-agent-pr]")).toBeNull();
});
