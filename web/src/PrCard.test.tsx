// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { docsProject } from "./docs/fixtures/project";
import { initializeInterfaceI18n } from "./i18n/instance";
import { PrCardView } from "./PrCard";
import type { ChangeRequestRef, HostDeclaration, PullRequest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

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

const NUMBER = 12;

/** A reference as the core's GitHub unit makes it for `url`'s repository (docs/ARCHITECTURE.md, Code hosts). */
function githubReference(authority: string, url: string): ChangeRequestRef {
  return { repository: { host: "github", authority, path: "acme/app" }, number: NUMBER, key: `${authority}/acme/app#${NUMBER}`, label: `#${NUMBER}`, url, checks_url: `${url}/checks`, files_url: `${url}/files` };
}

const GITHUB: HostDeclaration = { key: "github", name: "GitHub", operations: ["merge", "update_branch", "edit_body", "merge_proof", "base_policy", "commit_checks", "failed_log", "by_number", "handover", "issues", "search", "watch"], access: { program: "gh", minimum_version: "2.40.0", sign_in: "gh auth login" } };

/** One open pull request of a docs project, whose card the core read, with `reason` deciding the card's first button. */
async function mountCard(url: string, reference: ChangeRequestRef, reason: "review" | "merge" = "merge") {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  initializeInterfaceI18n("en");
  const { rest, project, checkout } = docsProject();
  const pr: PullRequest = {
    number: NUMBER, title: "Retry a payment at most three times", url, reference, badge: "open", state: "pending", is_draft: false, review: null, checks: "pending",
    head_branch: "feat/retry", base_branch: "main", closing_issues: [],
    work: { author: "you", author_is_bot: false, mine: true, merge_state: "clean", review_requests: [], asked_of_me: "none", failed_checks: [], pending_checks: 1, queue: { state: "not_queued" }, review_requested_at_unix_ms: null },
  };
  checkout.branch = "feat/retry";
  checkout.pull_request = pr;
  checkout.github = { available: true, loading: false, stale: false, failure_category: null, last_success_at_unix_ms: Date.now(), host: GITHUB };
  project.pull_requests = [pr];
  project.agent_scope!.prs = { rows: [{ number: NUMBER, group: "needs_you", fold: null, reason: { kind: reason }, agents: [], agent_pane: null, checkout_id: checkout.id, checkout_ids: [checkout.id], issue: null }], groups: [{ group: "needs_you", numbers: [NUMBER] }], needs_you: 1 } as never;
  rest.pr_work = { link: null, feedback: null, cards: [{ workspace_id: project.id, number: NUMBER, host: GITHUB, reading: false, head: "a".repeat(40), body: null, files: [{ path: "src/retry.ts", additions: 3, deletions: 1, generated: false }], files_total: 1, threads: [], merge: { can_merge: true, methods: ["merge"], queue: false }, failed: [], write: null }] };
  useShellStore.setState({ rest, agents: [], connection: "live" });
  useUiStore.setState({ screen: { kind: "workspace" } });
  const open = vi.spyOn(window, "open").mockReturnValue(null);
  const actions = createActions(() => true);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><PrCardView workspaceId={project.id} number={NUMBER} actions={actions} front={false} /></TooltipProvider>));
  return { container, open, opened: () => open.mock.calls.map((call) => call[0]) };
}

const button = (container: HTMLElement, text: string) => [...container.querySelectorAll<HTMLButtonElement>("button")].find((each) => each.textContent?.includes(text))!;

// A6 of the GitHub behavior table: the card opens GitHub's own checks and changed files pages of the pull request,
// on github.com and on a GitHub Enterprise server whose address keeps its port.
for (const [authority, url] of [["github.com", "https://github.com/acme/app/pull/12"], ["ghe.example.com:8443", "https://ghe.example.com:8443/acme/app/pull/12"]] as const) {
  it(`opens the checks page and the changed files page of a pull request on ${authority}`, async () => {
    const reference = githubReference(authority, url);
    const review = await mountCard(url, reference, "review");
    await act(async () => button(review.container, "View all checks").click());
    await act(async () => review.container.querySelector<HTMLButtonElement>("[data-pr-card-review-github]")!.click());
    // The card remembers an opened files line per pull request while the page runs, so open it only while it is shut.
    if (!review.container.querySelector("[data-pr-card-row=files][data-open]")) await act(async () => review.container.querySelector<HTMLButtonElement>("[data-pr-card-toggle=files]")!.click());
    await act(async () => button(review.container, "src/retry.ts").click());
    expect(review.opened()).toEqual([`${url}/checks`, `${url}/files`, `${url}/files`]);
  });
}
