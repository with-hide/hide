// @vitest-environment jsdom
// A6 of the GitHub behavior table: what the screen opens for a GitHub project's repository and how it tells that two
// addresses name the same pull request, held before and after the screen stopped reading and making addresses.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import tokensText from "../../design/tokens.json?raw";
import { emptyScope, legacyRest } from "../test/legacyAgentScope";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { initializeInterfaceI18n } from "./i18n/instance";
import { Sidebar } from "./sidebar";
import type { ChangeRequestRef, Checkout, Device, GithubStatus, SnapshotRest, Workspace } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import type { DispatchFn } from "./ws";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
const savedShell = useShellStore.getState();
const savedUi = useUiStore.getState();
afterEach(() => {
  useShellStore.setState(savedShell, true);
  useUiStore.setState(savedUi, true);
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

const LOCAL: Device = { id: "local", label: "This Mac", kind: "local", state: "local", message: null, ssh_alias: null, agent_count: 0, test: null, agent_scope: emptyScope() };

/** A reference as the core's GitHub unit makes it (docs/ARCHITECTURE.md, Code hosts): the key folds the path's case. */
function reference(path: string, number: number): ChangeRequestRef {
  const url = `https://github.com/${path}/pull/${number}`;
  return { repository: { host: "github", authority: "github.com", path }, number, key: `github.com/${path.toLowerCase()}#${number}`, label: `#${number}`, url, checks_url: `${url}/checks`, files_url: `${url}/files` };
}

function project(id: string, repository: string, status: GithubStatus | undefined): Workspace {
  const checkout: Checkout = { id: `${id}-main`, workspace_id: id, path: `/work/${id}`, label: id, branch: "main", purpose: null, is_worktree: false, exists: true, has_panes: false, pull_request: null, active_tab_id: null, strip: [], next_tab_label: "1", tabs: [], agent_scope: emptyScope(), github: status };
  return { id, label: id, path: `/work/${id}`, device_id: "local", registered: true, temporary: false, pinned: false, is_git: true, agent_scope: emptyScope(), inactive_checkouts: { expanded: false, checkout_ids: [] }, checkouts: [checkout], home_issues: { repository, issues: [], overflow: false } } as Workspace;
}

it("opens a GitHub project's repository page from its sidebar mark", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  const tokens = (JSON.parse(tokensText) as { tokens: Record<string, { value: number }> }).tokens;
  for (const name of ["--size-sidebar-ideal", "--size-sidebar-min", "--size-sidebar-max"]) document.documentElement.style.setProperty(name, `${tokens[name]?.value}px`);
  initializeInterfaceI18n("en");
  const status: GithubStatus = { failure_category: null, available: true, loading: false, stale: false, last_success_at_unix_ms: 1, repository: { name: "acme/app", url: "https://github.com/acme/app" } };
  const app = project("app", "acme/app", status);
  useUiStore.setState({ screen: { kind: "workspace" }, overviewOpen: false });
  useShellStore.setState({ connection: "live", agents: [], rest: legacyRest({ navigator: { devices: [LOCAL], workspaces: [app], focused_device_id: "local", focused_checkout_id: app.checkouts[0]!.id } }, []) });
  const open = vi.spyOn(window, "open").mockReturnValue(null);
  const actions = createActions(() => true);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><Sidebar actions={actions} /></TooltipProvider>));
  try {
    await act(async () => container.querySelector<HTMLButtonElement>("[data-project-github=app]")!.click());
    expect(open).toHaveBeenCalledWith("https://github.com/acme/app", "_blank", "noopener,noreferrer");
  } finally {
    await act(async () => root.unmount());
    for (const name of ["--size-sidebar-ideal", "--size-sidebar-min", "--size-sidebar-max"]) document.documentElement.style.removeProperty(name);
  }
});

it("opens a session's pull request on the card of the project that holds it, in any case, and another repository's in the browser", async () => {
  initializeInterfaceI18n("en");
  const held = reference("acme/app", 7);
  const other = reference("acme/other", 7);
  const pull = (ref: ChangeRequestRef) => ({ number: 7, url: ref.url, reference: ref });
  const workspaces = [
    { ...project("app", "acme/app", undefined), pull_requests: [pull(held)] },
    { ...project("other", "acme/other", undefined), pull_requests: [pull(other)] },
  ] as unknown as Workspace[];
  useShellStore.setState({ rest: { navigator: { siblings_shown: 5, focused_device_id: "local", devices: [LOCAL], workspaces } } as SnapshotRest });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  // The session printed the address in another case: GitHub reads a repository's path in any case.
  const printed = reference("Acme/App", 7);
  actions.openSessionPullRequest({ workspace_id: "other", number: 7, url: printed.url, reference: printed });
  expect(events.filter((event) => event.kind === "pr_open").map((event) => (event.payload as { workspace_id: string }).workspace_id)).toEqual(["app"]);
  const open = vi.spyOn(window, "open").mockReturnValue(null);
  const absent = reference("acme/absent", 7);
  actions.openSessionPullRequest({ workspace_id: "app", number: 7, url: absent.url, reference: absent });
  expect(open).toHaveBeenCalledWith("https://github.com/acme/absent/pull/7", "_blank", "noopener,noreferrer");
});
