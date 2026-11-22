// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { docsProject } from "./docs/fixtures/project";
import { clientI18n } from "./i18n/translator";
import { initializeInterfaceI18n } from "./i18n/instance";
import { LinkTimeline } from "./LinkSessions";
import type { HostDeclaration, LinkPanel, PullRequest } from "./snapshot";
import { useShellStore } from "./store";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
const savedShell = useShellStore.getState();
afterEach(() => {
  useShellStore.setState(savedShell, true);
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

const GITHUB: HostDeclaration = { key: "github", name: "GitHub", operations: ["merge", "update_branch", "issues", "handover", "search", "watch"], access: { program: "gh", minimum_version: "2.40.0", sign_in: "gh auth login" } };
const GITLAB: HostDeclaration = { key: "gitlab", name: "GitLab", operations: ["commit_checks"], access: { program: "glab", minimum_version: "1.78.0", sign_in: "glab auth login --hostname gitlab.com" } };

/** The timeline of an open pull request no session has worked on yet, in a project `host` read. */
async function mountTimeline(host: HostDeclaration, language: "en" | "ko") {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  initializeInterfaceI18n("en");
  await clientI18n.changeLanguage(language);
  const { rest, project, checkout } = docsProject();
  checkout.github = { available: true, loading: false, stale: false, failure_category: null, last_success_at_unix_ms: Date.now(), host };
  const pr = { number: 12, title: "Retry a payment", state: "pending", badge: "open", head_branch: "feat/retry", base_branch: "main" } as PullRequest;
  const panel: LinkPanel = { workspace_id: project.id, target: { kind: "pr", number: 12 }, loading: false, failure: null, pr: null, prs: [], total: 0, sessions: [] };
  useShellStore.setState({ rest, agents: [], connection: "live" });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><LinkTimeline panel={panel} project={project} pr={pr} milestones={[{ kind: "opened", at: 1 }]} branchOf={() => "feat/retry"} onRetry={() => undefined} actions={createActions(() => true)} /></TooltipProvider>));
  return {
    none: container.querySelector("[data-link-none]")?.textContent ?? "",
    opened: container.querySelector("[data-timeline-milestone=opened]")?.textContent ?? "",
    delegate: container.querySelector("[data-link-delegate]") !== null,
  };
}

// docs/UI_BEHAVIOR.md, Code hosts: the timeline names the item in its host's word, and a hand-over the host does
// not offer is not drawn; a GitHub project's timeline reads as it did before hosts.
it("names a gitlab.com merge request MR and draws no hand-over, and keeps a GitHub pull request's words and hand-over", async () => {
  expect(await mountTimeline(GITHUB, "en")).toMatchObject({ none: expect.stringContaining("No sessions have worked on this PR"), opened: expect.stringContaining("PR opened"), delegate: true });
  expect(await mountTimeline(GITHUB, "ko")).toMatchObject({ none: expect.stringContaining("이 PR에서 일한 세션이 없어요"), opened: expect.stringContaining("PR 열림"), delegate: true });
  expect(await mountTimeline(GITLAB, "en")).toEqual({ none: "No sessions have worked on this MR", opened: expect.stringContaining("MR opened"), delegate: false });
  expect(await mountTimeline(GITLAB, "ko")).toEqual({ none: "이 MR에서 일한 세션이 없어요", opened: expect.stringContaining("MR 열림"), delegate: false });
});
