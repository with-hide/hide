// Flow 9's world: two invented projects, one whose remote is on GitHub and one
// on gitlab.com, each with an agent at work. The core read both, so the
// sidebar's PR list holds GitHub's pull request beside the gitlab.com
// project's merge requests, each project's checkout names the host that read
// it (`GithubStatus.host`), and each pull request carries the reference its
// host made (`ChangeRequestRef`). The world answers the sidebar's lists and a
// row opening its card on the desk, as the core does on `pr_open`. Settings ›
// Remote repositories draws a row per host from what the projects carry, so it
// needs nothing more. Nothing leaves the page: every address is an example.

import type { ChangeRequestRef, DetailItem, HostDeclaration, PrCard, PullRequest, SidebarLens, Workspace } from "../../snapshot";
import { nextSidebarLens } from "../../snapshot";
import type { CoreEvent, PracticeWorld, WorldSnapshot } from "../world";
import { agent, applyDesk, cardDesk, closedCard, deskSnapshot, oneTabDesk, shellScreen, type DeskModel, type PracticePlace } from "./kit";

const GITHUB_PLACE: PracticePlace = { project: "web", label: "web", branch: "main" };
const GITLAB_PLACE: PracticePlace = { project: "shop-api", label: "shop-api", branch: "main" };
const HOUR_MS = 3_600_000;

/** The hosts as the core declares them: GitHub with its operations, gitlab.com with the one it offers while it is read only. */
const GITHUB: HostDeclaration = {
  key: "github", name: "GitHub",
  operations: ["merge", "update_branch", "edit_body", "merge_proof", "base_policy", "commit_checks", "failed_log", "by_number", "handover", "issues", "search", "watch"],
  access: { program: "gh", minimum_version: "2.72.0", sign_in: "gh auth login" },
};
const GITLAB: HostDeclaration = {
  key: "gitlab", name: "GitLab", operations: ["commit_checks"],
  access: { program: "glab", minimum_version: "1.78.0", sign_in: "glab auth login --hostname gitlab.com" },
};

/** The merge request the learner opens, and the other one the gitlab.com project has open. */
export const MR_NUMBER = 12;
const OTHER_MR = 14;
/** GitHub's pull request, beside them in the same list. */
const PR_NUMBER = 41;
/** The list row of the merge request the learner opens. */
export const MR_ROW = `[data-pr="${MR_NUMBER}"][data-pr-project="${GITLAB_PLACE.project}"]`;
/** Its card, once open on the desk. */
export const MR_CARD = `[data-pr-card="${MR_NUMBER}"][data-pr-card-project="${GITLAB_PLACE.project}"]`;

/** What the world can change: the two desks, which list the sidebar shows, and the cards open on the desk. */
export type GitlabModel = { desk: DeskModel; other: DeskModel; lens: SidebarLens; detail: DetailItem[] };

/** Whether the sidebar shows its PR list. */
export const showsPrs = (model: GitlabModel) => model.lens === "prs";
/** Whether the merge request's card is the one in front of the desk. */
export const showsMr = (model: GitlabModel) => {
  const top = model.detail.at(-1);
  return top?.kind === "pull_request" && top.workspace_id === GITLAB_PLACE.project && top.number === MR_NUMBER;
};

function initial(): GitlabModel {
  const lead = agent({ pane_id: "lead", identity_label: "Retry a payment at most three times", agent_kind: "claude", group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "6m" });
  const web = agent({ pane_id: "web", identity_label: "Keep a named tab open", agent_kind: "codex", group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "12m" });
  return { desk: oneTabDesk({ id: "lead", agent: lead, title: "Retry a payment at most three times" }), other: oneTabDesk({ id: "web", agent: web, title: "Keep a named tab open" }), lens: "projects", detail: [] };
}

// --- The pull requests ---------------------------------------------------------

/** A reference as each host's unit makes it: the number written the host's way and its own pages, at an example address. */
function reference(host: "github" | "gitlab", path: string, number: number): ChangeRequestRef {
  const authority = host === "github" ? "github.com" : "gitlab.com";
  const url = host === "github" ? `https://example.com/${path}/pull/${number}` : `https://example.com/${path}/-/merge_requests/${number}`;
  const [checks, files] = host === "github" ? ["checks", "files"] : ["pipelines", "diffs"];
  const sign = host === "github" ? "#" : "!";
  return { repository: { host, authority, path }, number, key: `${authority}/${path}${sign}${number}`, label: `${sign}${number}`, url, checks_url: `${url}/${checks}`, files_url: `${url}/${files}` };
}

type Spec = { host: "github" | "gitlab"; path: string; number: number; title: string; branch: string; state: PullRequest["state"]; failed: number; mine: boolean; hoursAgo: number };

const SPECS: Spec[] = [
  { host: "gitlab", path: "acme/shop-api", number: MR_NUMBER, title: "Retry a payment at most three times", branch: "payment-retry", state: "failed", failed: 1, mine: true, hoursAgo: 1 },
  { host: "gitlab", path: "acme/shop-api", number: OTHER_MR, title: "Cache the price list for a minute", branch: "price-cache", state: "pending", failed: 0, mine: false, hoursAgo: 5 },
  { host: "github", path: "acme/web", number: PR_NUMBER, title: "Keep a named tab when its last agent closes", branch: "named-tab", state: "mergeable", failed: 0, mine: true, hoursAgo: 3 },
];

function pullRequest(spec: Spec, nowMs: number): PullRequest {
  const ref = reference(spec.host, spec.path, spec.number);
  const counts = { total: 2, passed: 2 - spec.failed, failed: spec.failed, running: 0, waiting: 0, skipped: 0, unknown: 0 };
  return {
    number: spec.number, title: spec.title, url: ref.url, reference: ref, badge: spec.mine ? "open" : "review", review: spec.mine ? "approved" : "review_required", is_draft: false,
    checks: spec.failed ? "failing" : "passing", state: spec.state,
    created_at_unix_ms: nowMs - (spec.hoursAgo + 2) * HOUR_MS, updated_at_unix_ms: nowMs - spec.hoursAgo * HOUR_MS, head_branch: spec.branch, base_branch: "main", closing_issues: [],
    work: {
      author: spec.mine ? "you" : "teammate", author_is_bot: false, mine: spec.mine, merge_state: spec.failed ? "blocked" : "clean",
      review_requests: spec.mine ? [] : [{ kind: "user", name: "you" }], asked_of_me: spec.mine ? "none" : "direct",
      failed_checks: spec.failed ? [{ name: "unit-test", url: `${ref.checks_url}` }] : [], pending_checks: 0, check_counts: counts, queue: { state: "not_queued" },
      review_requested_at_unix_ms: spec.mine ? null : nowMs - spec.hoursAgo * HOUR_MS,
    },
  } as PullRequest;
}

/** The core's board for one project: the failed merge request and GitHub's mergeable pull request in Needs You, the one asked of the learner as a review. */
function board(specs: Spec[], nowMs: number): Workspace["agent_scope"]["prs"] {
  const reason = (spec: Spec) => (spec.failed ? { kind: "failed" as const, name: "unit-test" } : { kind: spec.mine ? "merge" as const : "review" as const });
  const rows = specs.map((spec) => ({
    number: spec.number, group: "needs_you" as const, fold: null, reason: { ...reason(spec), since_unix_ms: nowMs - spec.hoursAgo * HOUR_MS }, agents: [], agent_pane: null, checkout_id: null, checkout_ids: [], issue: null,
  }));
  return { rows, groups: [{ group: "needs_you", numbers: specs.map((spec) => spec.number) }], needs_you: specs.length } as never;
}

/** The card the core read for the merge request: its pipeline failed on one job, and its files; no merge, which gitlab.com does not offer here. */
function mrCard(): PrCard {
  return {
    workspace_id: GITLAB_PLACE.project, number: MR_NUMBER, host: GITLAB, reading: false, head: "0000012",
    body: "A payment that times out is tried again, at most three times, before the order is marked failed.",
    files: [{ path: "src/payment/retry.ts", additions: 31, deletions: 0, generated: false }, { path: "src/payment/retry.test.ts", additions: 24, deletions: 0, generated: false }],
    files_total: 2, threads: [], merge: null, failed: [], write: null,
  } as PrCard;
}

// --- The snapshot ----------------------------------------------------------------

/** A project as the core sends it once its host read it: the host and repository on its checkout, its pull requests and their board. */
function read(project: Workspace, host: HostDeclaration, path: string, nowMs: number): Workspace {
  const specs = SPECS.filter((spec) => spec.path === path);
  const github = { available: true, loading: false, stale: false, failure_category: null, last_success_at_unix_ms: nowMs - 30_000, host, repository: { name: host.key === "gitlab" ? `gitlab.com/${path}` : path, url: `https://example.com/${path}` } };
  return {
    ...project,
    checkouts: project.checkouts.map((checkout) => ({ ...checkout, github })),
    pull_requests: specs.map((spec) => pullRequest(spec, nowMs)),
    agent_scope: { ...project.agent_scope, prs: board(specs, nowMs) },
  };
}

function snapshot(model: GitlabModel, nowMs: number): WorldSnapshot {
  const base = deskSnapshot([{ place: GITLAB_PLACE, model: model.desk }, { place: GITHUB_PLACE, model: model.other }], GITLAB_PLACE.project, nowMs);
  const navigator = base.rest.navigator!;
  const workspaces = navigator.workspaces!.map((project) => (project.id === GITLAB_PLACE.project ? read(project, GITLAB, "acme/shop-api", nowMs) : read(project, GITHUB, "acme/web", nowMs)));
  return {
    ...base,
    rest: {
      ...base.rest,
      navigator: { ...navigator, workspaces },
      ui_state: { ...base.rest.ui_state!, sidebar_lens: model.lens },
      pr_work: { link: null, feedback: null, cards: [mrCard()] },
      // A card opened onto the desk opens the lane, as the core does on `pr_open`.
      workspace_view: cardDesk({ ...base.rest.workspace_view!, open: model.detail.length > 0 }, model.detail),
    },
  };
}

// --- The events ------------------------------------------------------------------

function apply(model: GitlabModel, event: CoreEvent): GitlabModel | null {
  const p = event.payload;
  switch (event.kind) {
    case "sidebar_lens_set": {
      const lens = (p.lens as SidebarLens | undefined) ?? nextSidebarLens(model.lens);
      return lens === model.lens ? null : { ...model, lens };
    }
    // ⇧⌘R and the 이슈 없는 PR line: the sidebar turns to the list it names.
    case "sidebar_project_set": {
      const lens = p.lens as SidebarLens | undefined;
      return !lens || lens === model.lens ? null : { ...model, lens };
    }
    // A row of either project opens its card; only the merge request has one the core read.
    case "pr_open": {
      const id = p.workspace_id;
      if ((id !== GITLAB_PLACE.project && id !== GITHUB_PLACE.project) || typeof p.pr_number !== "number") return null;
      return { ...model, detail: [...model.detail, { kind: "pull_request", workspace_id: id, number: p.pr_number }] };
    }
    case "view_layout": {
      const index = closedCard(event);
      return index === null || index >= model.detail.length ? null : { ...model, detail: model.detail.filter((_, at) => at !== index) };
    }
    default: {
      const desk = applyDesk(model.desk, event);
      return desk ? { ...model, desk } : null;
    }
  }
}

/** What each pane's terminal shows: the agent fixing the merge request's pipeline, or a shell. */
function paneScreen(model: GitlabModel, paneId: string): string {
  const pane = [...model.desk.panes, ...model.other.panes].find((row) => row.id === paneId)!;
  if (!pane.agent) return shellScreen(GITLAB_PLACE);
  return [
    "\x1b[38;5;209m✻\x1b[0m Claude Code",
    "",
    `\x1b[2m>\x1b[0m ${pane.title}`,
    "",
    "\x1b[38;5;209m●\x1b[0m The unit-test job failed: the third retry was not counted.",
    "  \x1b[2m⎿  Reading src/payment/retry.ts…\x1b[0m",
    "",
    "\x1b[38;5;209m✻\x1b[0m \x1b[2mFixing the count… (esc to interrupt)\x1b[0m",
  ].join("\r\n");
}

export const GITLAB_WORLD: PracticeWorld<GitlabModel> = {
  initial,
  apply,
  snapshot,
  panes: (model) => [...model.desk.panes, ...model.other.panes].map((pane) => pane.id),
  screen: paneScreen,
  examples: ["[data-pr-title]", "[data-pr-card-title]"],
};
