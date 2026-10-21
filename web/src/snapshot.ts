import type { AgentScope } from "./agentScope";
import type { ProjectSettings, ProjectSettingsWrite } from "./projectSettings";
// The parts of the core's `rest` section the web shell reads, typed as the
// core serializes them (herdr-core/src/model.rs), plus the pure selectors that
// resolve the operator's focused checkout, its visible tab and that tab's
// layout. Every selector is a lookup: the snapshot carries every tab's layout,
// so a tab switch never draws a waiting state (PRD S2 B3).

import type { AgentLogoMode, UsagePreferences } from "./generated/hided-ws";

/** The status codes the core sends in `status_code`; the catalogs name one word for each (agents.status.*). */
export type AgentStatusCode = "error" | "question" | "approval" | "working" | "done" | "idle" | "unknown" | "waiting" | "unfinished" | "stopped" | "attached" | "sleeping" | "waking" | "sleep_failed";

/** Whose move is next (`RowMark`): the operator's, in progress (the agent works, or `waiting` on proven work, D-25), unread as finished, unread otherwise, nothing (resting), a state Hide cannot read, or asleep (status-marks D-21). */
export type RowMark = "turn" | "going" | "waiting" | "done" | "unread" | "quiet" | "unknown" | "asleep";

/** What a quiet row waits on (`AgentWait`): its descendants, a background job its session proves is running, the reply to a Hide request it sent, or its pull request's checks. */
export type AgentWait = "children" | "background" | "reply" | "checks";

/** Why a row's second line is on screen (`RowLine::mode`); see agentRow.ts. */
export type LineMode = "request" | "news" | "waiting" | "kept" | "quiet" | "vanished" | "checks_passed" | "checks_failed";

export type StatusTone = { kind: "warning" | "working" | "success" | "subtle" | "news"; read: boolean };

/** Agent decisions are projected by herdr-core::agent_state, never reconstructed by the shell. */
export type AgentState = {
  session: { group: SessionGroup; line: string | null; unfinished: boolean };
  attention: boolean; needs_you: boolean; root: boolean;
  title_emphasized: boolean; selection_emphasizes_title: boolean;
  asking: boolean; working: boolean; waits_on_children: boolean;
  chip_tone: StatusTone; mark_tone: StatusTone;
  line: { text: string; mode: LineMode; tone: StatusTone; pull_request?: number } | null;
  branch_badge: string | null;
  bucket: "turn" | "working" | "delegating" | "resting";
  attention_rank: number;
  search_tone: "attention" | "working" | "done" | "muted";
  subtree: "working" | "waiting" | "unread" | "unknown" | "quiet";
  link: "working" | "question" | "idle"; link_rank: 0 | 1 | 2;
  verb: RequestVerb; request_todo: boolean; descendant_asking: number; request_since: number | null;
  /** A Needs You row's second line: a verb and what to do; `more` counts the other asks under the same root. */
  ask: { verb: RaiseVerb; what: string | null; more: number } | null;
  /** Where the row stands among its siblings in a tree: asking, working, finished unread, the rest. */
  tree_rank: number;
  /** The row's own PRs, worst first; never its descendants'. `index` points into `request.pull_requests`. */
  pr: { count: number; worst: PrState; pulls: { index: number; state: PrState }[] } | null;
};

/** What the operator does about a raised descendant, in lead order (`escalation::Verb`). */
export type RaiseVerb = "approval" | "answer" | "confirm" | "draft";
/** The one state every PR mark draws, worst first (`model::PrState`, docs/status-model.md, Pull request visual states). */
export type PrState = "failed" | "pending" | "mergeable" | "draft" | "merged" | "closed";

/** One raised descendant as its lineage root shows it (`escalation::RaisedAsk`). */
export type RaisedAsk = {
  verb: RaiseVerb;
  what: string | null;
  /** The raised descendant itself, where a tree draws the ask. */
  raised_pane_id: string;
  /** The raised descendant's title; drafts may name its parent as the holder. */
  origin_title?: string;
  /** The agent the band names: the raised descendant, or for a draft the parent holding it. */
  pane_id: string;
  title: string;
  agent_kind: string;
  open_pane_id: string;
  since_unix_ms: number | null;
  unreceived_by?: string;
  path: string[];
  checkout: string | null;
  human_notice: boolean;
};

/** What Hide AI started an agent for, as codes the screen words (`memory_promote`, the card `m72`). */
export type HideAiTask = { kind: string; target?: string };

/** The screen that owns what a Hide AI task does (PRD hide-ai-work-home D-10); a kind the core has no home for carries none. */
export type HideAiHome = "memory";

/** What the home waits for the operator to do, beyond what the agent itself asks (D-14); the core decides it. */
export type HomeTurn = { kind: "pr_review" | "ci_failed"; pr_number: number };

/**
 * Hide AI started this agent (PRD hide-ai-origin-mark D-01); `task` is absent for a start that named none.
 * With a `home` the row belongs to that screen: it is listed only while the operator has something to do,
 * `workspace_id` names the catalog Project whose home shows it and `turn` what that home waits for.
 */
export type HideAi = { task?: HideAiTask; home?: HideAiHome; workspace_id?: string; turn?: HomeTurn };

/** A checkout Hide AI made for an agent it started, joined to a checkout by device and path. */
export type HideAiCheckout = { device_id: string; path: string; task?: HideAiTask };

export type AgentRow = {
  /** Set by the core from the coordination record of the spawn, never from a branch name. */
  hide_ai?: HideAi;
  resolved?: { at_unix_ms: number; source: "operator" | "automatic" } | null;
  /** Resolved within the last 24 hours, so Sessions still lists it. */
  resolved_recent?: boolean;
  /** The operator's ★ or a read, unanswered ask, and since when it waits (PRD revisit-group). */
  revisit?: { starred: boolean; read_ask: boolean; since_unix_ms: number } | null;
  escalation?: { cause: "parent_blocked" | "draft" | "bell_exhausted" | "undelivered" | "child_blocked" | "observer_unconfirmed"; letter_id: string | null; since_unix_ms: number | null; human_notice: boolean } | null;
  /** How many letters wait on this pane for a possible draft, once it has rested; absent while none waits (docs/delivery.md). */
  letters_waiting?: number;
  /** On a lineage root: every raised descendant, lead first. */
  raised?: RaisedAsk[];
  /** The one mark a folded parent wears for its descendants: raised, else working, else none. */
  descendant_mark?: { kind: "raised" | "working"; count: number } | null;
  state: AgentState;
  id: string;
  pane_id: string;
  identity_label: string;
  /** Conversation display provenance, independent of lifecycle and hook health. */
  record_connection?: "checking" | "confirmed" | "unconfirmed";
  record_summary?: string;
  /** The name Herdr knows the agent by (`agent start --name`, `agent rename`); absent when it was given none. */
  herdr_name?: string;
  agent_kind: string;
  /** The checkout that physically owns this pane. */
  checkout_label?: string | null;
  /** Presentation-only device facts attached by `allAgents`. */
  device_id?: string;
  device_label?: string;
  mark: RowMark;
  group: string;
  status_code: AgentStatusCode;
  detail?: string | null;
  /** Everything the agent last said through its hooks, uncut: the request, then the progress, one per line; absent when it said nothing. */
  message?: string | null;
  /** Native unanswered question/plan, bounded and session/state-proven by the core. */
  user_turn?: UserTurnFact;
  /** When the core last saw this agent change state (epoch ms), retained for state ordering. */
  changed_at_unix_ms: number | null;
  /** Latest actual native request in the proven session (epoch ms). Absent when unknown; never fall back to state observation time. */
  last_prompt_at_unix_ms?: number | null;
  /** The core's ordering key: `changed_at_unix_ms` as thirteen digits, else Herdr's state sequence zero-padded, so it sorts as text. */
  last_activity?: string;
  emphasized: boolean;
  unread: boolean;
  requires_close_confirmation?: boolean;
  requires_close_status_check?: boolean;
  unknown?: boolean;
  demand?: string;
  activity?: string;
  /** Work another agent delegated; ordinary children are Working or Seen, with core escalation as the exception (docs/status-model.md). */
  delegated?: boolean;
  lineage_parent_pane_id?: string | null;
  /** Present on a fork the operator made with Fork agent: where it came from (PRD fork-operator-root). */
  forked_from?: ForkSource | null;
  /** The ancestors from the lineage root down to the parent, root first. */
  lineage_path_pane_ids?: string[];
  lineage_child_pane_ids?: string[];
  /** Every live descendant pane in the order closing this row takes them, deepest first; absent when none (PRD close-agent-subtree D-20). */
  close_descendant_pane_ids?: string[];
  /** What every live descendant is doing, counted by state; unknown activity is in none. */
  descendant_counts?: DescendantCounts;
  direct_child_counts?: DescendantCounts;
  /** Why a quiet row waits instead of being done or stopped: drawn as a ring in Working (docs/status-model.md); absent when it does not. */
  wait?: AgentWait;
  /** The background tasks a finished turn left running, which its line ends with (status-marks D-19); absent when none. */
  background_left?: number;
  /** How deep under its lineage root; a root is 0. */
  lineage_depth?: number;
  /** Whether the operator has this row's descendants folded away; folded is the default. */
  lineage_collapsed?: boolean;
  /** The checkout this row runs in, set only when it differs from its parent's. */
  lineage_worktree_badge?: string | null;
  /** Present only while Hide holds this agent asleep (PRD agent-sleep); absent when awake. */
  sleep?: AgentSleep;
  /** The conversation id Herdr recorded for this agent; absent when it recorded none. */
  session_id?: string | null;
  /** The agent searches its own conversation, so ⌘F asks the core where the search goes (`pane_find_open`). */
  own_find?: boolean;
  /** The request view's block (`AgentRequestSnapshot`); absent on a row the core has not laid it on. */
  request?: AgentRequest;
};

export const USER_TURN_KINDS = ["plan_approval", "question"] as const;
export type UserTurnFact = {
  kind: typeof USER_TURN_KINDS[number];
  content: { text: string; choices: string[]; truncated: boolean } | null;
};

/** How the label read a turn's end (`LabelEnd`). */
export type LabelEnd = "working" | "question" | "blocked" | "done" | "waiting" | "unfinished";

/** What a row asks of the operator now (`RequestVerb`), in the order the request view draws its groups. */
export type RequestVerb = "answer" | "blocked" | "fix" | "review" | "stopped" | "result" | "working" | "waiting" | "idle";

export type SessionGroup = "needs_you" | "working" | "done" | "idle" | "resolved";

/** Who sent the request a row shows (`RequestSender`). */
export type RequestSender = { kind: "operator" } | { kind: "named"; name: string } | { kind: "agent" };

/** The request view's part of an agent row (`AgentRequestSnapshot`, PRD overview-request-view). */
export type AgentRequest = {
  verb: RequestVerb;
  /** When the row took this verb; kept across a restart (D-40). */
  verb_since_unix_ms: number;
  /** The label's line for the turn (B18, B47); absent with summaries off or no analysis, when the reply stands in (D-12). */
  line?: string;
  /** How the label read the turn's end (`contracts/snapshot-wire-enums.json`: `label_end`). */
  end?: LabelEnd;
  /** The operator's last request, else the last one another agent sent. */
  request: { text: string; cut: boolean; images: number; at_unix_ms: number; sender: RequestSender } | null;
  /** Who sent a request after the operator's last one (B4). */
  later_by: RequestSender | null;
  /** The agent's last words. */
  reply: { text: string; cut: boolean; at_unix_ms: number } | null;
  /** The chip first (D-46), then the other live ones, then settled ones. */
  pull_requests: AgentPullRequest[];
};

/** One of a row's pull requests (`AgentPullRequestSnapshot`). */
export type AgentPullRequest = {
  number: number;
  title: string;
  url: string;
  badge: PullRequest["badge"];
  checks: NonNullable<PullRequest["checks"]>;
  review?: PullRequest["review"];
  head_branch: string;
  closing_issues: IssueReference[];
  /** Drawn as the row's chip or counted in its `+N` (D-43). */
  live: boolean;
  /** This row holds the pull request's duty (D-31). */
  duty: boolean;
  /** The row's session made it, as opposed to its branch having it. */
  created: boolean;
  settled_at_unix_ms: number | null;
};

/** A sleeping agent's state (`AgentSleepSnapshot`): the row and the pane draw it. */
export type AgentSleep = {
  state: "sleeping" | "waking" | "failed";
  /** Why the last wake failed, in plain words or as a start refusal code (`startReason`); set only when `failed`. */
  reason?: string | null;
  /** When it fell asleep, in Unix milliseconds. */
  since_unix_ms: number;
  /** The last progress line it reported before it slept. */
  progress?: string | null;
};

/**
 * Where a fork came from (`ForkSourceSnapshot`): the source's pane and title while
 * the source runs the conversation it was forked from; both absent once it closed
 * or moved on, and the mark then names no pane and opens nothing.
 */
export type ForkSource = { pane_id?: string | null; label?: string | null };

/**
 * What the pane menu and header know about forking this pane (`PaneForkSnapshot`):
 * `available` offers Fork agent and Fork agent to the side, `reason` says why a
 * forkable agent cannot fork yet, and `forked_from` is present on a fork.
 */
export type PaneFork = { available: boolean; reason?: string | null; forked_from?: ForkSource | null };

/** Where the pane menu's fork puts the fork: its own tab, or beside the source. */
export type ForkPlacement = "tab" | "side";

/** The pane menu's Sleep agent item (`AgentSleepActionSnapshot`); absent where it does not apply. */
export type AgentSleepAction = { available: boolean; reason?: string | null };

export type DescendantCounts = { error: number; approval: number; question: number; working: number; done: number };

/** One agent in a line of them: a pane header chip or a lineage step's sibling (`AgentChipSnapshot`). */
export type AgentChip = {
  tone: StatusTone;
  pane_id: string;
  label: string;
  checkout_label?: string | null;
  detail: string | null;
  status_word_visible: boolean;
  agent_kind: string;
  demand: string;
  activity: string;
  emphasized: boolean;
  mark: RowMark;
  status_code: AgentStatusCode;
  delegated: boolean;
};

/** What a pane's agent delegated (`PaneChildrenSnapshot`); absent on a pane with no agent. */
export type PaneChildren = {
  instrumented: boolean;
  uninstrumented_reason: string | null;
  uninstrumented_label: string | null;
  chips: AgentChip[];
  /** Whether Hide hears this pane's session; absent for an agent with no connection to judge. */
  connection?: PaneConnection | null;
};

/** Why a session is not connected: `codex_shared_server`, `started_before_hide`, or `setup_needed` (fix it in Settings, no Reopen). */
export type PaneConnectionReason = "codex_shared_server" | "started_before_hide" | "setup_needed";

/** `PaneConnectionSnapshot`: `reason` is present exactly when `connected` is false; `reopen` is the last Reopen of the pane. */
export type PaneConnection = {
  connected: boolean;
  /** Whether the popover offers Reopen: false for `setup_needed` and for a pane on another device. */
  can_reopen: boolean;
  reason: PaneConnectionReason | null;
  reopen: { state: "pending" } | { state: "failed"; reason: PaneReopenFailure } | null;
};

export type PaneReopenFailure = "start_refused" | "session_gone" | "codex_unread" | "agent_busy" | "end_refused";

/** One step of a pane's lineage, root first and ending at the pane itself. */
export type LineageStep = { pane_id: string; label: string; siblings: AgentChip[] };

/** The core's outcome of the latest pane focus that carried a request id. */
export type PaneFocusRequest = {
  request_id: string;
  target_pane_id: string;
  phase: "pending" | "succeeded" | "failed" | string;
  message: string | null;
  retryable: boolean;
};

export type PullRequest = {
  number: number;
  title: string;
  url: string;
  badge: "merged" | "closed" | "review" | "open";
  review: "review_required" | "changes_requested" | "approved" | null;
  is_draft: boolean;
  /** The CI rollup; unknown and absent checks never read as a pass (`PullRequestChecks`). */
  checks?: "unknown" | "none" | "pending" | "failed" | "passing";
  merged_at_unix_ms?: number | null;
  updated_at_unix_ms?: number | null;
  /** When GitHub made it: the timeline's `PR 열림` row (PRD pr-card-and-session-record-redesign B5). */
  created_at_unix_ms?: number | null;
  /** The branch the pull request comes from. */
  head_branch?: string;
  /** The issues its body closes when it merges, as GitHub reads the body. */
  closing_issues?: IssueReference[];
  /** The one state its mark draws, decided by the core (`PrState::of`). */
  state: PrState;
  /** The branch it merges into. */
  base_branch?: string;
  /** The GitHub values the PRs tool's list and the card read (`PullRequestWork`). */
  work?: PullRequestWork;
};

export type MergeState = "unknown" | "clean" | "has_hooks" | "unstable" | "dirty" | "behind" | "blocked" | "draft";

/** The values `gh` gave beyond badge, checks and review (`PullRequestWork`). */
export type PullRequestWork = {
  author: string | null;
  author_is_bot: boolean;
  /** The signed-in account wrote it; null while the account could not be read. */
  mine: boolean | null;
  merge_state: MergeState;
  review_requests: { kind: "user" | "team"; name: string }[];
  /** `unknown` while the question that tells could not be answered. */
  asked_of_me: "unknown" | "none" | "direct" | "team";
  failed_checks: { name: string; url: string | null }[];
  pending_checks: number;
  /** Disjoint counts from the open head's GitHub rollup; absent until read. */
  check_counts?: { total: number; passed: number; failed: number; running: number; waiting: number; skipped: number; unknown: number } | null;
  /** Its place in the base branch's merge queue; `unknown` while the queue could not be read. */
  queue: { state: "unknown" | "not_queued" } | { state: "at"; position: number };
  review_requested_at_unix_ms: number | null;
};

/** What a pull request's card reads beyond the list, per head commit (`PrCardSnapshot`). */
export type PrCard = {
  workspace_id: string;
  number: number;
  reading: boolean;
  head: string | null;
  body: string | null;
  files: { path: string; additions: number; deletions: number; generated: boolean }[] | null;
  files_total: number;
  threads: { reviewer: string | null; path: string; line: number | null; text: string }[] | null;
  merge: { methods: PrMergeMethod[]; queue: boolean; can_merge: boolean } | null;
  /** Parts the last read could not get. */
  failed: ("body" | "files" | "threads")[];
  /** When the head's last check run finished, for the timeline's check row (PRD pr-card-and-session-record-redesign D-15); absent until the card read it or when no check ran. */
  checks_completed_at_unix_ms?: number;
  /** When the latest approval or change request was submitted, for the timeline's review row; absent until read or without one. */
  review_at_unix_ms?: number;
  write: PrWrite | null;
};

export type PrMergeMethod = "merge" | "squash" | "rebase";

/** The Merge or branch update asked for from a card, and how it went (`PrWriteSnapshot`). */
export type PrWrite = {
  action: "merge" | "update_branch";
  head: string;
  method: PrMergeMethod | null;
  phase: "working" | "done" | "failed";
  reason: "head_moved" | "denied" | "conflict" | "not_mergeable" | "unreachable" | "failed" | null;
};

/** Why a `gh` lookup failed, as the core names it; the catalogs word each (issueSettings.gh*). */
export type GithubFailureCategory = "not_installed" | "not_logged_in" | "no_github_remote" | "network_or_rate_limit";

/** How a repository's `gh` lookup is doing, apart from what it found (`GithubStatusSnapshot`). */
export type GithubStatus = {
  failure_category: GithubFailureCategory | null;
  available: boolean;
  loading: boolean;
  stale: boolean;
  last_success_at_unix_ms: number | null;
  unavailable_reason: string | null;
  /** The extra questions of the last read that went unanswered while the pull request list was read (`ExtraQuery`). */
  unread_queries?: ExtraQuery[];
};

/** A question the pull request read asks beside the list: the signed-in account, the review requests, the merge queue. */
export type ExtraQuery = "account" | "review_requests" | "merge_queue";

export type IssueReference = { repository: string; number: number };

/** One GitHub issue as `gh` reported it (`IssueSnapshot`); `state` is `OPEN` or `CLOSED`. */
export type Issue = {
  reference: IssueReference;
  title: string;
  url: string;
  state: string;
  project_status: string | null;
  updated_at_unix_ms: number | null;
  created_at_unix_ms: number | null;
  /** Absent when the issue has none; the web reads a task's labels, not these. */
  labels?: IssueLabel[];
};

/** The issue a checkout is linked to and where the link came from (`IssueLinkSnapshot`). */
export type IssueLink = { issue: Issue; source: string };

/** A project's open issues, capped by the core (`ProjectIssuesSnapshot`). */
export type ProjectIssues = { repository: string | null; issues: Issue[]; overflow: boolean };

/**
 * One task of a project's source, in the core's source-neutral shape
 * (`TaskSnapshot`, PRD task-agents-views D-14). The web reads no source's own
 * shape: `source` is `github` or `local`, and a later source is drawn the same way.
 */
export type Task = {
  key: string;
  source: string;
  /** The id the source shows (`#N`, `owner/repo#N` for another repository, `L-N` for a local issue), or null when it has none. */
  id: string | null;
  url: string | null;
  title: string;
  open: boolean;
  /** When the source last changed it, for the backlog's order and age. */
  updated_at_unix_ms?: number | null;
  created_at_unix_ms?: number | null;
  /** Actual closure time; editing a closed issue does not reset it. */
  closed_at_unix_ms?: number | null;
  /** The open tasks this one waits on, possibly of another project (`TaskRefSnapshot`). */
  blocked_by?: TaskRef[];
  /** GitHub's sub-issues of this task with its own progress count; absent when it has none (`TaskSubIssuesSnapshot`). */
  sub_issues?: TaskSubIssues | null;
  /** The source's labels in its order, read with the list; absent when it has none, as every Local issue (`TaskLabel`). */
  labels?: IssueLabel[];
  author?: string | null;
  assignees?: string[];
};

/** GitHub's `completed` of `total` sub-issues, and the sub-issues themselves. */
export type TaskSubIssues = { total: number; completed: number; items: TaskSubIssue[] };

/** One sub-issue, by the key a pull request's closing reference names it with. */
export type TaskSubIssue = { key: string; id: string | null; title: string; open: boolean };

/** Another task, by its key and the id this task's source shows for it. */
export type TaskRef = { key: string; id: string | null };

/** Where a project's tasks come from and how the last read went (`TaskSourceSnapshot`). */
export type TaskSource = {
  kind: string;
  label: string;
  name: string | null;
  /** No read has answered yet. */
  reading: boolean;
  /** The last read failed; the tasks are the answer before it. */
  failure: string | null;
  last_read_at_unix_ms: number | null;
  /** The operator chose this source from the project's Issue source menu rather than the default. */
  chosen?: boolean;
  /** A GitHub source's repository is public; absent when GitHub has not said (`공개일 수 있음`) and for Local. */
  public?: boolean;
};

/** A project's tasks (`ProjectTasksSnapshot`); `source` is null only for a device's project. */
export type ProjectTasks = {
  source: TaskSource | null;
  sources?: TaskSource[];
  tasks: Task[];
  overflow: boolean;
};

/** How starting work from an issue behaves (`IssueSettingsSnapshot`, Settings › Issues). */
export type IssueSettings = {
  ai_worktree_name: boolean;
  closes_instruction: boolean;
};

/**
 * The agent kind and each kind's model the last start chose (`ui_state.agent_start`,
 * PRD home-device-rail D-18): every start surface preselects them. Null kind and no
 * model mean the CLI's own default.
 */
export type AgentStartChoice = {
  kind: string | null;
  models: Partial<Record<string, string>>;
};

/** A label as the issue's source colours it (`TaskLabel`); `color` is six hex digits. */
export type IssueLabel = { name: string; color: string | null };

export type IssueComment = { author: string | null; created_at_unix_ms: number | null; body: string };

/**
 * One issue as its panel and the Start dialog read it (`IssueDetailSnapshot`):
 * the body, and for a GitHub issue its labels, author, assignees and latest
 * comments. The fields past `message` are empty until `ready`.
 */
export type IssueDetail = {
  workspace_id: string;
  device_id: string;
  task_key: string;
  phase: "reading" | "ready" | "failed";
  body: string | null;
  message: string | null;
  labels: IssueLabel[];
  author: string | null;
  created_at_unix_ms: number | null;
  assignees: string[];
  /** Absent for a source with no comments (Local). */
  comment_count: number | null;
  comments: IssueComment[];
  comments_cursor?: string | null;
  comments_has_more?: boolean;
  comments_limited?: boolean;
  comments_loading?: boolean;
};

/** The Overview's issue work in flight (`IssueWorkSnapshot`), one slot each. */
export type IssueWork = {
  /** Items the backlog added this run, by the web's intent key (PRD overview-device-tasks D-10, D-34). */
  backlog?: BacklogItem[];
  /** The last `되돌리기`. */
  undo?: BacklogUndo | null;
  /** Where each selected workspace/device/item came from (`backlog_origin`); kept while any window shows that owner. */
  origins?: BacklogOrigin[];
  /** Local flags belong to their device; GitHub repository/number flags have a null device (B27). */
  missed_keys?: { task_key: string; device_id: string | null }[];
  /** Every GitHub issue key Hide made for the backlog: only these and memos show `되돌리기` (D-21, B31). */
  made_keys?: string[];
  detail: IssueDetail | null;
  name: { request_id: string; phase: "working" | "ready" | "failed"; name: string | null; message: string | null } | null;
  /** The answer to a Local issue's edit, by the web's request id. */
  update?: { request_id: string; task_key: string; phase: "ready" | "failed"; message: string | null } | null;
  /** ⌘K's explicit GitHub search, by the web's request id (`github_search`). */
  search?: GithubSearch | null;
};

/** Why a backlog item was not made or not undone (`BacklogFailure`). */
export type BacklogFailure = "not_installed" | "not_logged_in" | "no_github_remote" | "network_or_rate_limit" | "queue_full" | "unconfirmed" | "repository_changed" | "unavailable" | "refused";

/** One item the backlog add made or is making (`BacklogItemSnapshot`). */
export type BacklogItem = {
  intent: string;
  workspace_id: string;
  title: string;
  phase: "waiting" | "creating" | "created" | "failed";
  /** The item's task key once it exists. */
  task_key: string | null;
  failure: BacklogFailure | null;
  /** Whether `backlog_retry` can act on this failed row; a refusal, or a repository that changed, cannot. */
  retry: boolean;
  /** The missed line it came from. */
  missed: { session: WorkSession; id: string } | null;
  at_unix_ms: number;
};

export type BacklogUndo = { workspace_id: string; device_id: string; task_key: string; phase: "working" | "done" | "failed"; failure: BacklogFailure | null };

export type BacklogOrigin = {
  workspace_id: string;
  device_id: string;
  task_key: string;
  phase: "reading" | "ready" | "failed";
  session: WorkSession | null;
  session_title: string | null;
  missed_text: string | null;
  task_name: string | null;
  request: string | null;
  answer: string | null;
};

/** One pull request or issue the core found on GitHub for ⌘K's search (`GithubSearchResultSnapshot`). */
export type GithubSearchResult = {
  kind: "pr" | "issue";
  repository: string;
  number: number;
  title: string;
  state: string;
  url: string;
  /** A pull request's one state, decided by the core with no checks or review read; null for an issue. */
  pr_state?: PrState | null;
};

/** The answer to the latest `github_search`: working while it runs, then ready with its results or failed. */
export type GithubSearch = {
  request_id: string;
  query: string;
  phase: "working" | "ready" | "failed";
  results: GithubSearchResult[];
  message: string | null;
};

/** A pull request linked to an issue (`PrLinkSnapshot`): the issue made, Hide's link, then `Closes #N` in the body. */
export type PrLink = {
  request_id: string;
  workspace_id: string;
  pr_number: number;
  /** The step working now, or the one that failed. */
  step: "create" | "link" | "body";
  phase: "working" | "ready" | "failed";
  issue_key: string | null;
  /** The id the source shows for the issue (`#12`, `L-3`). */
  issue_id: string | null;
  /** This request made the issue. */
  created: boolean;
  message: string | null;
};

/** A pull request's body, failed checks and standing change requests, read for a new issue made from it or an agent's first prompt (`PrFeedbackSnapshot`). */
export type PrFeedback = {
  request_id: string;
  pr_number: number;
  phase: "reading" | "ready" | "failed";
  /** The body once read. */
  body: string | null;
  failed_checks: { name: string; url: string | null }[];
  change_requests: { author: string | null; body: string }[];
  message: string | null;
};

/** The Overview's pull-request work in flight (`PrWorkSnapshot`), one slot each. */
export type PrWork = { link: PrLink | null; feedback: PrFeedback | null; cards?: PrCard[] };

export type Purpose = { text: string; origin: string };

/** A checkout's agents by state; `unknown` is a subset of `seen`, not a fifth group. */
export type CheckoutAgentSummary = {
  representative_pane_id: string | null;
  needs_you: number;
  done: number;
  working: number;
  seen: number;
  unknown: number;
  /** How many agents here draw each mark on their own row. */
  marks: MarkCounts;
};

/** How many rows draw each status mark (`MarkCountsSnapshot`); a quiet row draws none and is in none. */
export type MarkCounts = { turn: number; going: number; done: number; unread: number };

export type PaneRow = {
  terminal_id?: string | null;
  failed_terminal_id?: string;
  ports?: number[];
  servers?: { host: string; port: number }[];
  id: string;
  herdr_label: string | null;
  terminal_title: string | null;
  cwd: string;
  status_code: AgentStatusCode;
  requires_close_confirmation: boolean;
  requires_close_status_check: boolean;
  identity_label: string | null;
  children?: PaneChildren | null;
  lineage_path?: LineageStep[];
  sleep?: AgentSleep;
  sleep_action?: AgentSleepAction;
  fork?: PaneFork;
};

export type TabAgent = Pick<AgentRow, "agent_kind" | "mark" | "demand" | "activity" | "emphasized" | "wait" | "status_code"> & { state: Pick<AgentState, "mark_tone"> };

export type Tab = {
  /** `named_by_operator`: the manual name is one the operator gave through Rename, which keeps the tab when its agents leave. */
  sidebar?: { manual_name: string | null; number: number; named_by_operator: boolean; focused_pane_id?: string };
  agent?: TabAgent | null;
  id: string | null;
  workspace_id: string | null;
  checkout_id: string | null;
  label: string | null;
  empty: boolean;
  delegated: boolean;
  panes: PaneRow[];
};

export type StripTab = {
  id: string;
  kind: "herdr" | "file" | "diff" | "session";
  source_id: string;
  label: string;
  preview: boolean;
};

/** The removal gate the core computed for a linked worktree (`WorktreeDeletionGateSnapshot`). */
export type DeletionGate = {
  /** Main, locked or unavailable ignored-repository measurement. */
  blocked_reason: string | null;
  warnings: string[];
  button_label: string;
  can_delete_branch: boolean;
  /** What deleting the branch loses; present means the core deletes it with `git branch -D`. */
  branch_warning: string | null;
  /** The checkbox that accepts losing the folder's changes; present means deletion waits for it. */
  discard_label: string | null;
};

/** The parts of the core's worktree row the removal confirmation reads. */
export type WorktreeRow = {
  lock_reason?: string | null;
  ignored_repositories?: string[];
  ignored_scan_unavailable?: string | null;
  path: string;
  branch: string | null;
  head_sha: string | null;
  last_commit_unix_seconds?: number | null;
  is_main: boolean;
  missing: boolean;
  dirty: boolean;
  changed_file_count: number;
  /** Whether the branch is merged into its base; null when it could not be told. */
  merged?: boolean | null;
  /** Commits the upstream has that this branch does not, as of the last fetch; null when unread or no upstream. */
  behind_upstream?: number | null;
  pane_count: number;
  deletion_gate: DeletionGate;
  /** What the core measured of this checkout's folder; its `layers` split the size by what a build tool remakes. */
  disk?: WorktreeDisk;
};

/** One layer's share of a checkout: allocated bytes, how many folders, the biggest one's name (never a path). */
export type DiskCell = { bytes: number; folders: number; largest_name: string | null };

/** A checkout's allocated bytes split into the layers cleanup names; Git-visible files are `source_bytes`. */
export type DiskLayers = { build_cache: DiskCell; dependencies: DiskCell; other: DiskCell; source_bytes: number };

/** The two layers a cleanup may empty; `other` is size only (PRD disk-layers D-10). */
export type CacheLayer = "build_cache" | "dependencies";

export type WorktreeDisk = {
  path?: string | null;
  /** Null until measured and when a part could not be read. */
  total_bytes: number | null;
  unavailable_reason: string | null;
  measured_at_unix_ms?: number | null;
  largest_child_name?: string | null;
  largest_child_bytes?: number | null;
  layers?: DiskLayers | null;
  volume_free_bytes?: number | null;
};

export type Checkout = {
  agent_groups?: import("./agentLayout").CheckoutAgentGroups;
  agent_scope: AgentScope;
  id: string;
  workspace_id: string;
  label: string;
  path: string;
  branch: string | null;
  purpose: Purpose | null;
  is_worktree: boolean;
  /** Core-owned home choice; absent in snapshots from older daemons. */
  is_primary?: boolean;
  exists: boolean;
  /** A checkout Hide opened outside every registered project. */
  temporary?: boolean;
  has_panes: boolean;
  /** Who works here, counted by state, and the one agent that speaks for them (`CheckoutAgentSummary`). */
  agent_summary?: CheckoutAgentSummary;
  /** The worktree row behind a Git checkout, or null for a plain folder. */
  worktree?: WorktreeRow | null;
  /** The work done here is in the base: Git finds HEAD there and the link record ties its sessions to a merged pull request and to none still open. Git alone cannot say so, since a checkout with no commits of its own is in its base too. Absent from older daemons. */
  landed?: boolean;
  pull_request: PullRequest | null;
  /** The issue this checkout's work is linked to. */
  issue?: IssueLink | null;
  /** The key of the task in its project's `tasks` this checkout works on. */
  task_key?: string | null;
  /** The other tasks this checkout's pull request closes; each shows the same pull request. */
  closes_task_keys?: string[];
  github?: GithubStatus;
  changed_file_count?: number;
  /** Commits on this branch since its base. */
  ahead?: number;
  tabs: Tab[];
  active_tab_id: string | null;
  strip: StripTab[];
  next_tab_label: string;
};

export type Workspace = {
  agent_scope: AgentScope;
  id: string;
  label: string;
  path: string;
  device_id: string;
  /** Set for a project on a registered SSH device; local projects carry null. */
  remote_target_id?: string | null;
  /** False while the operator has this project's checkouts folded (`ui_state.collapsed_workspace_ids`). */
  expanded?: boolean;
  is_git?: boolean;
  default_branch?: string | null;
  branches?: string[];
  registered: boolean;
  temporary: boolean;
  pinned: boolean;
  /** The device's Hide Home (`~/hide`, PRD home-device-rail D-01): drawn as the Home row, never in the Projects list. */
  is_home?: boolean;
  last_activity_unix_ms?: number | null;
  /** What `Remove project…` would close, counted by the core (D-10). */
  removal?: { pane_count: number; running_agent_count: number };
  checkouts: Checkout[];
  inactive_checkouts: { expanded: boolean; checkout_ids: string[] };
  session_folds?: { empty: string[]; child_only: string[]; cleanup: string[]; empty_open: boolean; cleanup_open: boolean; open_prs: number };
  /** What the project's agents are doing, as the core decided it (`ProjectAgentPresenceSnapshot`, PRD sidebar-titlebar-refresh D-20). */
  agent_presence?: ProjectAgentPresence;
  /** The repository's open issues, in the shape older readers read. */
  home_issues?: ProjectIssues;
  /** The project's tasks, which the sidebar's 태스크 list and the Overview's backlog read. */
  tasks?: ProjectTasks;
  /**
   * The project's pull requests for the PRs tool and the PR card, one per branch:
   * every open one, and a merged one while its worktree is recorded here or
   * for 14 days after it merged (PRD overview-lenses-prs D-52). A local Git
   * project's only; absent while it has none.
   */
  pull_requests?: PullRequest[];
  /**
   * The project's allocated disk: a Git project's every worktree and shared
   * Git directory counted once, a folder project's folder. Filled by the
   * Projects tab's sweep (`disk_watch`); absent on a dialed device.
   */
  disk?: ProjectDisk;
  /** The project's disk cleanup: its review, its progress and its result, present on the project the last `cleanup_review` named. */
  cleanup?: DiskCleanup | null;
};

export type DiskCleanupPhase = "loading" | "review" | "removing" | "complete" | "failed";

/** Why a worktree cannot be removed by the sheet; the shell words each code. */
export type CleanupExclusionCode =
  | "main"
  | "locked"
  | "unavailable"
  | "missing"
  | "alias"
  | "current"
  | "contains_worktree"
  | "in_use"
  | "pane_open"
  | "main_unavailable"
  | "detached"
  | "dirty"
  | "not_merged"
  | "merge_unverified"
  | "nested_repository"
  | "unverified";

/** `unverified` is a checkout the core has no facts about, which is never read as idle. */
export type CleanupInUse = { code: "agent_working" | "agent_waiting" | "descendant_busy" | "process" | "port" | "unverified"; name: string | null; port: number | null };

export type CleanupRow = {
  path: string;
  branch: string | null;
  head: string | null;
  is_main: boolean;
  exclusion_code: CleanupExclusionCode | null;
  exclusion_count: number | null;
  in_use: CleanupInUse | null;
  /** How many live panes of the checkout close with it: finished agents and shells nothing here is using. */
  pane_count: number;
  /** The worktree's own outcome once confirmed: `skipped` when a recheck found it changed or busy, `failed` when Git refused. */
  result: "removed" | "skipped" | "failed" | null;
  /** Why a skipped or failed worktree stayed: an exclusion code, `changed`, `not_found`, `unverified`, `close_refused` (a pane would not close) or `remove_refused`. */
  result_code: CleanupResultCode | null;
  /** The allocated size of the worktree this run removed. */
  bytes: number | null;
};

export type CleanupResultCode = CleanupExclusionCode | "changed" | "not_found" | "unverified" | "close_refused" | "remove_refused";

export type CleanupCellSkip = "in_use" | "changed" | "tracked_files" | "nested_repository" | "symlink" | "not_found" | "unverified";

export type CleanupCellResult = {
  path: string;
  layer: CacheLayer;
  outcome: "removed" | "skipped" | "failed";
  bytes: number;
  folders: number;
  /** What kept a folder of the cell (or all of it): a recheck code, or `io` when a move failed. */
  reason_code: CleanupCellSkip | "io" | null;
};

export type DiskCleanup = {
  id: number;
  workspace_id: string;
  repository_root: string;
  phase: DiskCleanupPhase;
  main_head: string | null;
  message: string | null;
  /** Set when in-use could not be read (no Herdr connection, a failed process read): nothing is selectable. */
  usage_error: string | null;
  /** The in-use reads are in: cache cells may be ticked while worktree checks still run (phase `loading`). */
  usage_ready: boolean;
  free_bytes: number | null;
  progress: { done: number; total: number } | null;
  rows: CleanupRow[];
  cell_results: CleanupCellResult[];
  free_before: number | null;
  free_after: number | null;
};

export type ProjectDisk = {
  /** Known only once every part is measured. */
  total_bytes: number | null;
  unavailable_reason: string | null;
  /** The Projects tab's sweep is measuring it now. */
  measuring: boolean;
  /** The sweep will measure it: what is shown is old or missing. */
  queued?: boolean;
  /** Free space on the volume the project sits on, as of the last measurement. */
  free_bytes?: number | null;
  /** The size of that volume. */
  volume_total_bytes?: number | null;
  /** What the checkouts that could be measured add up to; the entrance shows it as a subtotal while `total_bytes` is null. */
  confirmed_bytes?: number | null;
  /** The layers of the checkouts that could be measured, summed; absent only while none was. */
  layers?: ProjectDiskLayers | null;
};

export type ProjectDiskLayers = { build_cache: number; dependencies: number; source: number; other: number; shared_git: number };

export type InactiveProjectGroup = { device_id: string; expanded: boolean; project_ids: string[] };

/**
 * `working` while an awake agent draws a mark, not quiet (the row's name is
 * heavy); `asleep` while every agent sleeps, with the newest sleep's time and
 * whether the operator opened the fold since (D-02, D-03, D-19).
 */
export type ProjectAgentPresence = { working: boolean; asleep?: { since_unix_ms: number; expanded: boolean } };

export type LayoutNode =
  | { type: "pane"; pane_id: string }
  | { type: "split"; direction: "right" | "down"; ratio: number; first: LayoutNode; second: LayoutNode };

export type PaneLayout = {
  workspace_id: string;
  tab_id: string;
  focused_pane_id: string;
  zoomed: boolean;
  root: LayoutNode;
};

export type PaneHeader = {
  working: boolean;
  /** Why the terminal is not live, drawn in the pane's body. */
  notice: PaneNotice | null;
  /** What waits on the operator in this pane, drawn on its second row. */
  band: PaneBand | null;
  /** The current pane agent's own live working sentence, selected by the core. */
  progress?: string | null;
};
export type PaneNotice = {kind: string; tone: "muted" | "warning" | "error"; reason: string | null; since_unix_ms: number | null; exit_code: number | null};
/** The pane's own approval, answer, block or stop, or a raised descendant's ask; `read` once the operator has looked. */
export type PaneBand = {kind: "approval" | "answer" | "blocked" | "stopped" | "raised"; reason: string | null; since_unix_ms: number | null; read: boolean; action: {pane_id: string; label: string} | null; more: number; raised?: RaisedAsk};

export type TerminalPane = {
  pane_id: string;
  closed: boolean;
  exit_code: number | null;
  transport_state: string;
  transport_message: string | null;
  /** Core transport identity; absent in older snapshots and fixtures. */
  transport_generation?: number;
  /** This client only observes the pane and Herdr did not move it for the last wheel; absent while false. */
  scroll_held_elsewhere?: boolean;
};

/** Where keys sent against a creation request go (PRD instant-pane-topology D-11). */
export type InputRequest = {
  request_id: string;
  state: "pending" | "ready" | "discarded";
  /** The pane a pending request's keys wait for while it has the keyboard: a fork's pane until its agent starts. */
  held_for?: string | null;
};

export type AsyncOperation = {
  related_scope_ids?: string[];
  id: string;
  kind: string;
  target_id: string;
  scope_id: string;
  phase: string;
  stage: string;
  message: string | null;
  retryable: boolean;
};

export type RecentClosedPending = {
  key: string;
  target_id: string;
  label: string;
  phase: string;
  checking: boolean;
  message: string | null;
  retryable: boolean;
};

export type RecentClosed = {
  count: number;
  top_label: string | null;
  restoring: boolean;
  pending: RecentClosedPending[];
  can_reopen: boolean;
  reopen_blocked_reason: string | null;
};

/**
 * One entry of ⌘K's Recent list: the ids that open the checkout and the names
 * its row is drawn from while its device has no catalog to read them from.
 */
export type RecentCheckout = { device_id: string; checkout_id: string; project_name: string; branch: string };

/** The explorer's most recent filesystem change and how far it got. */
export type ExplorerOperation = {
  id: number;
  kind: string;
  phase: string;
  path: string;
  destination: string;
  message: string | null;
};

export type WorkspaceRegistration = {
  primary_checkout_id?: string | null;
  id: string;
  label: string;
  path: string;
  device_id: string;
  pinned: boolean;
  /** This registration is the device's Home (`~/hide`). */
  home?: boolean;
};

export type PaneFind = {
  pane_id: string | null;
  term: string;
  index: number;
  total: number;
  truncated: boolean;
  unavailable_reason: string | null;
  /** Where the last `pane_find_open` sent the search, named by the request that asked. */
  opened?: { request_id: string; route: PaneFindRoute };
};

/** `agent`: the agent's own search took the pane; `bar`: Hide's find bar opens (`contracts/snapshot-wire-enums.json`). */
export type PaneFindRoute = "agent" | "bar";

/** What kind of document the core decided an open file is (`files::open`). */
export type DocumentKind = "text" | "markdown" | "image" | "pdf" | "binary";

export type EditorTabKind = "file" | "diff" | "session";

/** One editor tab the core holds. Its id is the strip entry's `source_id`. */
export type EditorTabSnapshot = {
  id: string;
  workspace_id: string;
  checkout_id: string;
  path: string;
  label: string;
  kind: EditorTabKind;
  /** Which Changes group a diff tab shows; present only for diff tabs. */
  diff_committed: boolean | null;
  markdown_live: boolean;
  wrap: boolean;
  dirty: boolean;
  /** The checkout's one replaceable tab: a single click opens it, a double click promotes it. */
  preview: boolean;
  /** Why a View tab restored after a restart has no document (file gone, device unreachable); it only offers Close (S6 B20). */
  unavailable_reason?: string | null;
};

/** The disk state that makes a save a choice rather than a write: the draft's base revision and what the file holds now (null when it was removed). */
export type EditorConflictSnapshot = {
  opened_revision: string;
  disk_revision: string | null;
};

/** A save without a landed result: `saving`, `waiting` (for the device's helper), `unknown` (the answer was lost), `checking` (being read back), `refused` (refused or never sent) or `not_applied` (read back unchanged: not reached the file yet). */
export type EditorSaveSnapshot = {
  state: "saving" | "waiting" | "unknown" | "checking" | "refused" | "not_applied";
  message: string | null;
};

export type EditorDocumentSnapshot = {
  path: string;
  language: string | null;
  document_kind: DocumentKind;
  contents_utf8: string | null;
  /** The content revision (`sha256:<hex>`) the draft is based on; present for an editable document. */
  revision: string | null;
  dirty: boolean;
  /** Why an editable document takes no edits: its size or its permissions. */
  readonly_reason: string | null;
  conflict: EditorConflictSnapshot | null;
  save: EditorSaveSnapshot | null;
};

/** A file a device is still reading; its tab shows when the answer arrives. */
export type EditorOpeningSnapshot = { workspace_id: string; checkout_id: string; path: string };

/**
 * The core's editor section: every open document (one buffer per device,
 * checkout and document, S5.5) and the document of the active View area's
 * active display. The web shell reads documents from the `documents` section
 * instead (`DocumentsSection`); the wire's `document` field serves a
 * client without View areas and is always null here.
 */
export type EditorSnapshot = {
  tabs: EditorTabSnapshot[];
  active_tab_id: string | null;
  opening?: EditorOpeningSnapshot[];
};

/**
 * The documents the front Workspace's visible displays show (S7 contract
 * 3.1), a delta section of its own beside `editor`: `visible` names every
 * document on screen, `changed` carries only those past this reader's cursor.
 */
export type DocumentsSection = {
  visible: string[];
  changed: { tab_id: string; document: EditorDocumentSnapshot }[];
};

export type SidebarLens = "projects" | "tasks";
export type TaskView = "status" | "project";

/** One thing the lane's 보기 slot shows (`DetailItem`, PRD sidebar-task-lens D-50). */
export type DetailItem =
  | { kind: "pull_request"; workspace_id: string; number: number }
  | { kind: "record"; session: WorkSession };

/** The 보기 slot: what it opened, oldest first and the one it shows last, and whether the lane draws it. */
export type DetailSlot = { items: DetailItem[]; shown: boolean };

/** What a display shows now (S7 contract 3): its document, a read in flight, a root not readable yet, or a read that failed. */
export type ViewDisplayState = "open" | "opening" | "waiting" | "unavailable";

/** One place a file or diff is shown in a View area; several displays may show one document. */
export type ViewDisplaySnapshot = {
  id: string;
  /** The editor tab (document buffer) it shows; null while it is opening or waiting. */
  tab_id: string | null;
  /** The file or diff it shows; empty for a browser display. */
  path: string;
  /** The document's name, or a browser page's title (its host until it has one). */
  label: string;
  kind: "file" | "diff" | "browser";
  /** Which History group a diff shows; null for a file. */
  committed: boolean | null;
  preview: boolean;
  state: ViewDisplayState;
  /** Why it is waiting or unavailable. */
  reason: string | null;
  /** A browser display's address as the core last recorded it; absent for a file or diff. */
  url?: string | null;
  /** A browser display's page title as the core last recorded it. */
  title?: string | null;
  /** A browser display's load stamp: it moves when the core asks the page to load `url` (an open or a navigate). */
  load?: number | null;
  /** The agent that last opened it (PRD view-opener-feedback D-03); absent for a View the operator opened. */
  opener?: ViewOpener;
  /** Its unsent comments, and of those the ones whose letter expired (B14, B29); absent for none. */
  comments?: number;
  not_delivered?: number;
};

/** An agent that opened a View (`ViewOpenerSnapshot`). */
export type ViewOpener = {
  name: string;
  /** The agent's Herdr kind, for its logo and product name. */
  provider: string;
  device_id: string;
  /** The pane the agent is in now; absent while it is closed (B6). */
  pane_id?: string | null;
  /** Whether `Reopen and send` can start its session again (D-10). */
  resumable: boolean;
};

export type ViewRect = { x: number; y: number; width: number; height: number };

/** Where a comment points (`view_feedback::Anchor`). A region is in fractions of its image or PDF page. */
export type ViewCommentAnchor =
  | { kind: "lines"; start: number; end: number; lines: string[]; side?: "old" | "new" }
  | { kind: "element"; selector: string; text: string; rect: ViewRect; source?: string }
  | { kind: "area"; rect: ViewRect }
  | { kind: "region"; page?: number; rect: ViewRect };

export type ViewComment = {
  id: string;
  /** The displays of the layout it shows on. */
  display_ids: string[];
  kind: "file" | "diff" | "browser";
  path: string;
  url?: string;
  committed?: boolean;
  anchor: ViewCommentAnchor;
  note: string;
  shot: boolean;
  changed: boolean;
  not_delivered: boolean;
  revision?: string;
  /** The group it is sent with, `""` for a View no agent opened. */
  group: string;
  /** Its place in its group's letter, from 1 (B11). */
  number: number;
};

/** Everyone with open comments in the checkout (B30). */
export type ViewFeedbackGroup = {
  key: string;
  recipient?: ViewOpener;
  comments: number;
  views: number;
  not_delivered: number;
  display_ids: string[];
};

/** A View an agent opened that the layout shows, once per thing shown (B8). */
export type OpenedView = {
  pane_id: string;
  view_id: string;
  kind: "file" | "diff" | "browser";
  label: string;
  path: string;
  url?: string;
  comments: number;
};

/** Where a Send stands (`contracts/snapshot-wire-enums.json`: `view_feedback_send_state`). */
export type ViewFeedbackSendState = "sending" | "sent" | "waiting" | "reopening" | "failed";
/** Why a Send did not go (`contracts/snapshot-wire-enums.json`: `view_feedback_send_reason`). */
export type ViewFeedbackSendReason = "attachments_full" | "letter_unsaved" | "letters_full" | "shot_copy" | "reopen_failed" | "closed";

export type ViewFeedbackSend = {
  request_id: string;
  state: ViewFeedbackSendState;
  count: number;
  to: string;
  pane_id?: string;
  device?: string;
  reason?: ViewFeedbackSendReason;
  at_unix_ms: number;
};

/** The operator's comments on the front Workspace's Views (`ViewFeedbackSnapshot`). */
export type ViewFeedback = {
  comments: ViewComment[];
  groups: ViewFeedbackGroup[];
  opened: OpenedView[];
  send?: ViewFeedbackSend;
  /** The comment the last delete removed, which Undo brings back (B43). */
  deleted?: string;
};

export type ViewAreaSnapshot = import("./areaLayout").Area<ViewDisplaySnapshot>;
export type ViewSplitSnapshot = import("./areaLayout").AreaSplit<ViewDisplaySnapshot>;
export type ViewNode = import("./areaLayout").AreaNode<ViewDisplaySnapshot>;
export type ViewLayoutSnapshot = import("./areaLayout").AreaLayout<ViewDisplaySnapshot>;

export type DiffSnapshot = { path: string; committed: boolean; text: string; notice: string | null };

/** The six working-tree states the core presents (ChangedFileStatus). */
export type ChangedFileStatus = "modified" | "added" | "deleted" | "untracked" | "renamed" | "conflict";

export type ChangedFileSnapshot = {
  /** Absolute, so a row needs no second join against the root. */
  path: string;
  /** Relative to the checkout root, which is what the row shows. */
  relative_path: string;
  previous_relative_path: string | null;
  status: ChangedFileStatus;
  added_lines: number | null;
  removed_lines: number | null;
};

/**
 * One checkout's Git state. The core computes this only while the Changes or
 * Explorer surface is visible, so the web asks for it by sending that ui state
 * rather than by opening a second read path (PRD B1).
 */
export type ChangesSnapshot = {
  root_path: string | null;
  entries: ChangedFileSnapshot[];
  committed: ChangedFileSnapshot[];
  base_branch: string | null;
  selected_path: string | null;
  selected_committed: boolean;
  /** The selected diff of a client without View areas; the web shell reads `diffs`. */
  diff: { path: string; text: string; notice: string | null } | null;
  /** One bounded patch per visible diff display of the front Workspace (S7 contract 3.2); absent when none shows. */
  diffs?: DiffSnapshot[];
  unavailable_reason: string | null;
  /** The reason is only that Git finds no repository here: nothing to repair, so the Explorer draws no mark (issue 570). Absent when false. */
  not_a_repository?: boolean;
  /** Why the latest read failed while these entries, from the last good read, are still shown (S5.5 B22). */
  stale_reason?: string | null;
};

export type DeviceTestStage = { stage: string; state: string; detail: string };

/**
 * Where a device's file and Git work runs (`DeviceHostSnapshot`). consent is
 * `this_machine`, `none`, `granted` or `outdated`; state is `ready`,
 * `connecting`, `not_allowed`, `identity_changed`, `unsupported` or
 * `unavailable`, with message saying why and what to do.
 */
export type DeviceHost = {
  consent: "this_machine" | "none" | "granted" | "outdated";
  helper_root: string | null;
  /** The folder the device's `hide` command is linked in, named like `helper_root`. */
  cli_dir: string | null;
  contract: number;
  bound_identity: string | null;
  granted_at_unix_ms: number | null;
  state: "ready" | "connecting" | "not_allowed" | "identity_changed" | "unsupported" | "unavailable";
  message: string | null;
  platform: string | null;
  helper_path: string | null;
};

export type Device = {
  agent_scope: AgentScope;
  id: string;
  /**
   * The name the operator registered the device under; null on the core's
   * own row, which the shell names as its system does (`machineName`).
   */
  label: string | null;
  /** `local` for the daemon's own machine, `remote` for an SSH device. */
  kind: string;
  /** `local`, or for an SSH device `ready`, `unavailable` or `disabled`. */
  state: string;
  message: string | null;
  /**
   * Which trust or sign-in step refused the connection (S5.5 B38):
   * `host_key_changed`, `host_key_unknown` or `authentication`; null otherwise.
   */
  problem?: string | null;
  ssh_alias: string | null;
  /** The Herdr socket the registration names on the device; null reads its default server. */
  herdr_socket_path?: string | null;
  agent_count: number;
  test: { state: string; checked_at_unix_ms: number | null; stages: DeviceTestStage[] } | null;
  host?: DeviceHost;
  /** What Hide's install kit has put on this machine (`KitSnapshot`). */
  kit?: Kit;
  /** The machine's own name, when the core knows it (`DeviceSnapshot::machine_name`). */
  machine_name?: string | null;
  /** A node that dials the core rather than a device the core dials (`DeviceSnapshot::dials_in`). */
  dials_in?: boolean;
  /**
   * The device's Herdr server stays on an older release than Hide's pinned
   * Herdr (`DeviceSnapshot::herdr_behind`); null while it runs the pin or newer.
   */
  herdr_behind?: HerdrBehind | null;
};

/** Why a device's Herdr stays older than Hide's (`contracts/snapshot-wire-enums.json`: `herdr_behind_cause`). */
export type HerdrBehindCause = "cannot_hand_off" | "handoff_failed";

export type HerdrBehind = { server_version: string; version: string; cause: HerdrBehindCause };

/** One part of the install kit (`contracts/snapshot-wire-enums.json`: `kit_component_id`). */
export type KitComponentId = "cli" | "claude_code_hook" | "codex_hook" | "coordination_retirement";

/** What a part is on its machine (`contracts/snapshot-wire-enums.json`: `kit_component_state`). */
export type KitComponentState = "installed" | "outdated" | "not_installed" | "removed" | "failed" | "absent" | "off";

/** Whether an agent can be switched on where a machine runs (`contracts/snapshot-wire-enums.json`: `kit_agent_availability`). */
export type KitAgentAvailability = "available" | "not_installed" | "unsupported_system";

/** The first-run wizard's steps (`contracts/snapshot-wire-enums.json`: `first_run_step`). */
export type FirstRunStep = "agents" | "devices" | "notifications" | "project" | "done";

export type FirstRun = { version: number; step: FirstRunStep };
export type FirstRunProjectForm = { view?: "add" | "clone" | "create" | ""; url?: string; parent?: string; name?: string };
export type FirstRunChoices = { agents?: string[] | null; devices?: string[]; project_device?: string | null; project_path?: string; project_form?: FirstRunProjectForm; notifications_answered?: boolean };
export type HerdrProfile = {
  id: string; label: string; target: string; session: string; enabled: boolean;
  added_as: string | null; problem: string | null; device_id: string | null;
  connect_state: string; connect_error: string | null;
};
export type HerdrProfiles = { state: "idle" | "loading" | "ready" | "failed"; error: string | null; profiles: HerdrProfile[]; truncated: boolean };

/** The flow version this build draws; the core keeps the same number (`FIRST_RUN_VERSION`). */
export const FIRST_RUN_VERSION = 1;

/** The screens introduced once (`contracts/snapshot-wire-enums.json`: `screen_intro`). */
export type ScreenIntro = "factory";

/** One piece of an agent (its skill or its hook) on its machine. */
export type KitPiece = {
  state: KitComponentState;
  reason: string | null;
  location: string | null;
};

/**
 * One agent on one machine (`KitAgentSnapshot`): its switch, and what Hide put
 * there for it. `hook` is null for an agent that gets the skill only.
 */
export type KitAgent = {
  id: string;
  label: string;
  availability: KitAgentAvailability;
  enabled: boolean;
  /**
   * The machine's record holds the operator's own choice for this agent
   * (`KitAgentSnapshot.chosen`). An agent that is on only by default (Claude
   * Code, Codex) has none, so it is not kept under Installed once its
   * program is gone (B9).
   */
  chosen: boolean;
  skill: KitPiece;
  hook: KitPiece | null;
  /** Herdr's own integration for the agent; every supported agent has one, so null only from a device helper whose build predates the field. */
  herdr?: KitPiece | null;
  /** Hide does only some of what it does for Claude Code with this agent: the row wears the Partial chip, on or off. */
  partial?: boolean;
  /** Every feature of the Partial popover in the table's order (`hide_kit::Feature` ids); `supported` is what this build does. */
  features?: { id: KitFeatureId; supported: boolean }[];
  /** How many of the agent's sessions run on this machine now, a sleeping agent not counted; null for an agent whose sessions Hide does not read (Partial agents) and before the machine's sessions are read. */
  sessions?: number | null;
  doc_url: string;
};

export type KitFeatureId =
  | "skill"
  | "guidance"
  | "letters"
  | "bell"
  | "memory"
  | "spawn_guard"
  | "herdr_integration"
  | "sleep"
  | "fork"
  | "start"
  | "titles";

/** How the last turn-off of Codex's shared server on a machine ended (`CodexDaemonOffSnapshot`). */
export type CodexDaemonOff =
  | { state: "pending" }
  | { state: "done" }
  | { state: "failed"; reason: "codex_missing" | "codex_refused" | "timed_out" | "unreachable" | "stop_failed" };

export type KitComponent = {
  id: KitComponentId;
  label: string;
  state: KitComponentState;
  reason: string | null;
  location: string | null;
};

/**
 * A machine's install kit (PRD device-parity B7): the same parts on This Mac
 * and every device. `unavailable` says why the kit does not run there at all;
 * `offers_reinstall` is true only while a part needs it; `shares_account_with`
 * names another registered device that reaches the same account there.
 */
/** What stopped a machine's kit pass before it changed anything (`contracts/snapshot-wire-enums.json`: `kit_refusal_cause`). */
export type KitRefusalCause = "unsafe" | "record" | "busy" | "projects" | "other";

/** The variable that chose Hide's state folder (`kit_state_variable`), named so the operator knows what to change or unset. */
export type KitStateVariable = "HIDE_STATE_DIR" | "XDG_STATE_HOME";

/** A pass that stopped before changing anything; the kit's own words stay in the log. */
export type KitRefusal = {
  cause: KitRefusalCause;
  path: string | null;
  variable: KitStateVariable | null;
  /** With `variable`, on the machine whose app ran the pass: the command that stops its background service, which reads the variable only when it starts. */
  stop?: string | null;
};

export type Kit = {
  unavailable: string | null;
  busy: boolean;
  /** The machine's last pass or read stopped before changing anything; its parts and agents are then empty. */
  refused?: KitRefusal | null;
  components: KitComponent[];
  /** Every agent Hide has an adapter for; empty until the machine was checked. */
  agents: KitAgent[];
  /** A kit read asked with Check again is under way on this machine (B10). */
  checking?: boolean;
  /** Why the machine's last read failed, a code; null when it did not (B10). */
  check_failed?: string | null;
  offers_reinstall: boolean;
  shares_account_with: string | null;
  /** What the machine's Codex answered about its shared daemon: true has it, false is older, null no answer. */
  codex_daemon?: boolean | null;
  /** Whether that Codex starts the shared server on its own now: true is the reason a pane reads not connected; null no answer or an older Codex. */
  codex_daemon_on?: boolean | null;
  /** The last turn-off of the shared server on this machine; absent until one was asked. */
  codex_daemon_off?: CodexDaemonOff | null;
};

/** One pane's rectangle in a remote tab, as fractions of the tab's area (`RemotePaneLayoutFrame`). */
export type RemotePaneFrame = { pane_id: string; x: number; y: number; width: number; height: number };

/** A remote tab's geometry: Herdr reports rectangles, not the local split tree (`RemotePaneLayoutSnapshot`). */
export type RemotePaneLayout = {
  workspace_id: string;
  tab_id: string;
  focused_pane_id: string;
  zoomed: boolean;
  frames: RemotePaneFrame[];
};

/**
 * What the core projected from one SSH device's Herdr (`RemoteSessionSnapshot`).
 * Every id is scoped to the target (`remote:<target>:workspace:…`, `…:tab:…`,
 * `…:pane:…`), so a remote id can never name a local pane, and the focus
 * fields are that Herdr's own.
 */
export type RemoteSession = {
  workspaces: Workspace[];
  agents: AgentRow[];
  active_tab_ids: Record<string, string>;
  focused_workspace_id: string | null;
  focused_checkout_id: string | null;
  focused_tab_id: string | null;
  focused_pane_id: string | null;
  pane_layouts: RemotePaneLayout[];
};

export type RemoteStatus = {
  target_id: string;
  /** `connected`, `not_connected`, `stale`, `disabled`, `socket_missing`, or a failure word. */
  state: string;
  message: string | null;
  herdr_version: string | null;
  /** The last session read from that host; kept while `stale`. */
  session?: RemoteSession | null;
  /** How far the device's helper has confirmed its projects (`device_catalog`). */
  catalog?: DeviceCatalog;
  /** Since when the device has not answered; absent while it is connected (Overview B4). */
  unreachable_since_unix_ms?: number;
};

/**
 * `resolving`: the helper is being asked; `ready`: every directory is
 * confirmed; `unavailable`: the helper cannot be asked and `message` says why.
 * An unconfirmed directory is listed as its Herdr workspace alone.
 */
export type DeviceCatalog = {
  state: "resolving" | "ready" | "unavailable";
  message: string | null;
  refused: { path: string; message: string }[];
};

export type HerdrStatus = {
  state?: string;
  socket_path?: string | null;
  message?: string | null;
  expected_protocol?: number | null;
  received_protocol?: number | null;
  received_version?: string | null;
};

export type EnvironmentStatus = {
  key: string;
  required: boolean;
  format: string;
  state: string;
  absent_behavior: string;
  message: string;
};

export type CoreDiagnostic = { kind: string; message: string; occurred_at: number };

export type AiProvider = {
  id: string;
  label: string;
  /** The id the logos key the same agent by: the install kit's adapter id (`claude-code`, `codex`, `grok`, `opencode`, `pi`, `antigravity`). */
  agent: string;
  /** `ready`, `needs_login`, `usage_limited`, `not_installed`, `unavailable`, `unsupported`, or `unread`. */
  state: string;
  headline: string;
  /** The provider layer's own reason code (`cannot_guarantee_read_only`, `model_not_offered:<model>`), never prose. */
  message: string | null;
  /** The agent's program was found. */
  installed: boolean;
  /** Runs on and Add agent offer it: installed, signed in, a backend, and a call that cannot change files. */
  selectable: boolean;
  /** When a usage limit ends (unix ms), when the agent said. */
  retry_at_ms: number | null;
  /** Every current provider checks sign-in; false or absent remains readable from older peers. */
  login_checked?: boolean;
  /** The model it is asked for; empty is the CLI's own default. */
  model: string;
  models: string[];
  /** The list is documented rather than queried, as an older peer may report. */
  models_fixed: boolean;
  /** "CLI default" can be chosen: the agent is asked with no `--model`. */
  cli_default: boolean;
  /** Why the list is empty: a code (`not_asked`, `not_observed`, or the provider's own). */
  models_unavailable_reason: string | null;
  /** The weekly usage the toolbar read for this agent; null when it gave no figure. Absent from an older daemon. */
  weekly?: AiWeekly | null;
  /** The figure is at or past the usage limit, so Hide AI does not ask this agent (the core decides). Absent from an older daemon. */
  past_limit?: boolean;
};

/** One agent's weekly usage: a percent and when the week resets (unix ms), when the agent said. */
export type AiWeekly = { used_percent: number; resets_at_ms: number | null };

/** The agents tried, in this order, when Runs on cannot answer. */
export type AiFallback = { provider: string; model: string };

/** Why Runs on is not answering, and who answers instead; absent while it answers. */
export type AiRefusal = {
  provider: string;
  /** `usage_limited`, `past_limit`, `needs_login`, `not_installed`, `unavailable` or `unsupported`. */
  reason: string;
  /** When a usage limit or the week ends (unix ms), when the agent said. */
  retry_at_ms: number | null;
  /** The listed agent answering instead; null when none can. */
  using: string | null;
  /** With `past_limit`: the agent's weekly percent and the limit it is past. */
  used_percent?: number | null;
  limit?: number | null;
};

/** One concrete Host of `~/.ssh/config`: where `ssh -G` says it leads and the device that already uses it. */
export type SshHost = {
  alias: string;
  /** `user@host:port`; null when it could not be resolved (`problem` says why). */
  address: string | null;
  /** The name of the registered device using this alias or reaching the same address. */
  added_as: string | null;
  problem: "ssh_missing" | "ssh_failed" | "timed_out" | null;
};

/** `idle` before the first `ssh_hosts_list`, `loading` while one runs (the last answer stays), `ready` once it is in. */
export type SshHosts = {
  state: "idle" | "loading" | "ready";
  hosts: SshHost[];
  /** The config names more concrete aliases than are listed. */
  truncated: boolean;
};

export type BackgroundAi = {
  /** Use Hide AI; off, no model is asked anything. Absent from an older daemon, which is on. */
  enabled?: boolean;
  /** Runs on; null when nobody chose and none can be chosen yet. */
  provider: string | null;
  chosen: boolean;
  /** The `에이전트 요약` switch; absent from an older daemon, which always summarizes. */
  agent_summary?: boolean;
  /** The share of an agent's week Hide AI may use (Settings › Hide AI › Usage limit); null is No limit. Absent from an older daemon, which has none. */
  usage_limit?: number | null;
  providers: AiProvider[];
  fallback?: AiFallback[];
  refusal?: AiRefusal | null;
  unavailable_reason: string | null;
};

/** Each machine's hook parts are its kit rows (`Device.kit`); this section keeps what only the panes say. */
export type AgentHooks = {
  last_report_failure: string | null;
};

/** The core's one task slot: a worktree creation, an agent start, a purpose write. */
export type TaskOperation = {
  id: number;
  /** The device the task runs on; null is the daemon's own machine. */
  device_id?: string | null;
  kind: string;
  /** `working`, `ready` or `failed`. */
  phase: string;
  repository_root: string | null;
  branch: string | null;
  base_branch: string | null;
  path: string | null;
  pane_id: string | null;
  agent_kind: string | null;
  message: string | null;
  /** `starting`, `started`, `failed` or `unknown`; null when no agent was chosen. */
  agent_phase: string | null;
  agent_message: string | null;
  /** The id the start surface sent (`agent_start_in_checkout.request_id`), so each surface reads only its own answer. */
  request_id?: string | null;
};

/**
 * The core's one repository clone, from Add a project's Clone from URL. The
 * URL is never here (it can carry credentials); `host` is what is shown.
 */
export type RepositoryClone = {
  id: number;
  host: string;
  /** The folder the clone creates, canonical; the registration carries the same path. */
  path: string;
  /** `cloning`, `cancelling`, then `finished`, `failed` or `cancelled`. */
  phase: string;
  /** Git's own stage, `Receiving objects` and the like. */
  stage: string | null;
  percent: number | null;
  message: string | null;
};

/** One worktree deletion: `checking` before any close, `closing` panes, `removing` on the core's worker, then `finished` or `failed`. */
export type WorktreeRemoval = {
  id: number;
  /** The device the worktree is on; null is the daemon's own machine. */
  device_id?: string | null;
  repository_root: string;
  checkout_path: string;
  branch: string | null;
  delete_branch: boolean;
  force_delete_branch?: boolean;
  discard_changes?: boolean;
  phase: string;
  message: string | null;
};

/** A card's kind (`contracts/snapshot-wire-enums.json`: `memory_card_kind`). */
export type MemoryCardKind = "rule" | "decision" | "lesson" | "fact" | "preference";

/** Where a card's promotion into the repository stands (`memory_promotion_state`). */
/**
 * `building`: the card's decision is not built yet and another branch builds it; that branch's pull request carries the card in.
 * `unbuilt`: nothing builds it, and `물어볼 것` asks once.
 */
export type MemoryPromotionState = "running" | "pr_open" | "merged" | "failed" | "unconfirmed" | "building" | "unbuilt";

/** What the Memory tab's operator asks for (`memory_action`). */
export type MemoryAction = "enable" | "disable" | "delete" | "forget" | "undo" | "promote" | "dismiss" | "card_right" | "doc_right";

/** One card of the opened Project, as its two-line row shows it. */
export type MemoryRow = {
  number: number;
  title: string;
  kind: MemoryCardKind;
  /** At most 512 characters, only for cards in an active promotion. */
  body_preview?: string;
  human_source_count: number;
  /** Sessions the card was given to in full. */
  provided_session_count: number;
  /** Times an agent read it with `hide memory show`. */
  pull_count: number;
  updated_at_unix_ms: number;
  /** Waiting to be checked against the repository again; agents are not given it meanwhile (`확인 중`). */
  verifying: boolean;
  /** Null for an ordinary card, a dismissed one, or one whose pull request closed unmerged. */
  document_comparison?: MemoryDocument | null;
  promotion: {
    state: MemoryPromotionState;
    /** The durable promotion attempt's start; absent on older daemons. */
    started_at_unix_ms?: number;
    /** The agent putting it in the repository, while it runs, or the one at work on the branch that builds a `building` card; `pane_id` is null once the agent ended. */
    agent: { name: string; pane_id: string | null } | null;
    pr_number: number | null;
    /** Original repository-qualified target; unknown on legacy attempts. */
    pr_url?: string | null;
    target_path: string | null;
    /** Why GitHub could not say whether an `unconfirmed` attempt's pull request came, when it said. */
    github_failure: GithubFailureCategory | null;
  } | null;
};

/** What `물어볼 것` asks: a card to promote (with its reasons) or a promotion that ended with no pull request. */
export type MemoryQuestion =
  | { kind: "promote"; number: number; title: string; human_session_count: number; pulled_session_count: number }
  | { kind: "docs_disagree"; number: number; title: string; document: MemoryDocument }
  | { kind: "failed"; number: number; title: string; start_failure: MemoryStartFailure | null }
  /** The card's decision is not built and nothing builds it; `reason` is what the agent placing it saw. */
  | { kind: "unbuilt"; number: number; title: string; reason: string | null };

/**
 * Why a failed promotion's agent never started (`memory_start_failure_code`):
 * `detail` is the line its new pane's shell stopped at, for `shell_starting`.
 */
export type MemoryStartFailure = { code: "shell_starting" | "agent_not_started" | "card_not_processed" | "other"; detail: string | null };

export type MemoryDocument = { authority: "outdated" | "card_newer" | "ask"; document: string; document_at_unix_ms: number | null; operator_at_unix_ms: number | null };
export type MemoryDocumentComparison = MemoryDocument & { document_text: string; operator_text: string; source_session: string | null };

/** The card shown in full (`memory_expand`). */
export type MemoryCard = {
  document_comparison?: MemoryDocumentComparison | null;
  number: number;
  body: string;
  cited_paths: string[];
  sources: { provider_label: string; at_unix_ms: number | null; human: boolean }[];
  /** The text the current one replaced, which Undo brings back. */
  previous: { body: string; changed_at_unix_ms: number } | null;
};

/** One opened Project's Memory (`memory.project`), keyed by the catalog Project a shell opened. */
export type ProjectMemory = {
  workspace_id: string;
  /** Why this Project has no Memory here; null when it has. */
  unavailable: { reason: "unregistered" } | { reason: "unreadable" } | null;
  loading: boolean;
  enabled: boolean;
  capacity_reached: boolean;
  rows: MemoryRow[];
  questions: MemoryQuestion[];
  /** `memory_analysis_state`. */
  analysis: "idle" | "analyzing" | "ai_not_ready" | "checking" | "check_budget_exhausted";
  consent_required: boolean;
  usable_cards: number;
  checking_card: number | null;
  check_resume_at_unix_ms: number | null;
  /** The card just forgotten, offered back with Undo. */
  forgotten: number | null;
  pending_action: MemoryAction | null;
  preparing_hooks?: boolean;
  action_failure?: MemoryAction | null;
  expanded: MemoryCard | null;
  search: { query: string; numbers: number[] } | null;
};

/** The `memory` section: absent until a shell opens a Project's Memory. */
export type MemorySection = { project: ProjectMemory | null };

/** Where a pull request's issue link comes from: GitHub's closing reference or Hide's branch link. */
export type LinkIssueSource = "closes" | "hide";
/** `created`: the session printed the pull request's address when GitHub made it. */
export type LinkSessionRole = "created" | "worked";
/** Whether the session's own file is still there; `unknown` for a device's file. */
export type LinkFileState = "present" | "missing" | "unknown";
export type LinkTarget = { kind: "pr"; number: number } | { kind: "issue"; key: string };

/** One session line of a link panel: a session, or several joined by continuation (`ids`, oldest first). */
export type LinkedSession = {
  agent: string;
  id: string;
  ids: string[];
  device_id: string;
  role: LinkSessionRole;
  /** The pull request the line belongs to. */
  pr: number;
  request: string | null;
  started_at_unix_ms: number | null;
  ended_at_unix_ms: number | null;
  path: string | null;
  cwd: string | null;
  file: LinkFileState;
  parent: { name: string; agent: string; session_id: string; available: boolean } | null;
};

/**
 * The link record for the one pull request or issue a panel shows (PRD
 * link-graph D-45), on its own section of the wire. Failures are codes the
 * shell words.
 */
export type LinkPanel = {
  workspace_id: string;
  target: LinkTarget;
  loading: boolean;
  failure: string | null;
  pr: {
    number: number;
    branch: string;
    title: string;
    url: string;
    created_at_unix_ms: number | null;
    closed_at_unix_ms: number | null;
    merged_at_unix_ms: number | null;
    /** The one state the record can say (`PrState::of`): merged, closed, or open and pending. */
    state: PrState;
    issues: { key: string; source: LinkIssueSource }[];
    /** Recorded worktree paths for the branch, newest first. */
    worktrees: string[];
  } | null;
  /** The pull requests an issue target is linked to, newest first. */
  prs: number[];
  sessions: LinkedSession[];
  /** Every line the record holds for the target; `sessions` carries at most 200. */
  total: number;
};

/** Each Project's link counts and session chips by workspace id. */
export type LinkSummaries = {
  projects: Record<string, {
    prs?: Record<string, number>;
    issues?: Record<string, number>;
    sessions?: Record<string, { number: number; created: boolean }[]>;
    work?: SessionWorkSummary[];
    creators?: { source: WorkSession; kind: string; repository: string; number: number }[];
  }>;
  /** Session files are still being read for the first time. */
  filling: boolean;
  /** Each live pane session's tasks and children; a session with no task is absent. */
  progress?: WorkProgress[];
};

/** Native identity retained by the link record, including ended sessions. */
export type WorkSession = { device_id: string; agent: string; session_id: string };
export type WorkRequest = { offset: number; at_unix_ms: number; text: string; author: "operator" | "parent"; task_ids: string[]; chapter_id: string | null; judged: boolean };
export type WorkEnd = "working" | "question" | "blocked" | "done" | "waiting" | "unfinished";
/** `unjudged`: the session's newest turn went unjudged and the task is not done, so `end` is an earlier turn's (PRD task-continuation D-09). */
export type WorkTask = { id: string; chapter_id: string; name: string; at_unix_ms: number; last_at_unix_ms: number; end: WorkEnd; done: boolean; unjudged: boolean };
export type MissedWork = { id: string; kind: "leftover" | "problem" | "follow_up"; text: string; status: "open" | "resolved" | "issue" | "dismissed" | "backlog"; at_unix_ms: number; turn_offset: number; issue_repository: string | null; issue_number: number | null; backlog_key: string | null; creating: boolean };
export type WorkOutput = { kind: "pr" | "issue"; repository: string; number: number; title: string; url: string; state: string; stale: boolean; last_read_at_unix_ms: number | null; created_at_unix_ms: number; task_id: string | null; source: WorkSession; source_name: string };
export type SessionRecord = {
  session: WorkSession; title: string; fallback_title: string; native_title: string | null; manual_title: string | null; state: string | null; last_end: WorkEnd | null;
  cwd: string | null; path: string | null; started_at_unix_ms: number | null; ended_at_unix_ms: number | null;
  chapters: { id: string; goal: string; at_unix_ms: number }[]; tasks: WorkTask[]; requests: WorkRequest[]; missed: MissedWork[]; outputs: WorkOutput[]; children: SessionRecord[];
  /** How its direct children stand; absent without children. */
  child_progress?: ChildProgress | null;
  /** For a child, the task group of the parent's turn that spawned it (PRD task-continuation B7); null when that turn held no task. */
  spawn_task?: string | null;
};
/** One direct child as its parent counts it (`contracts/snapshot-wire-enums.json`: `child_work`): a resting child's report or merged pull requests, else its own tasks, else its last END; `unjudged` has no judgment that says, or a newest turn that went unjudged. */
export type ChildWork = "done" | "working" | "other" | "unjudged";
/** One segment per direct child; `done` is null while any child is unjudged, never a false zero. */
export type ChildProgress = { children: ChildWork[]; done: number | null };
/** A live pane's chip and checklist: its tasks first asked first, their pull requests and its children. */
export type WorkProgress = { session: WorkSession; done: number; total: number; tasks: WorkTask[]; outputs: WorkOutput[]; children: ChildProgress | null };
export type SessionWorkSummary = { session: WorkSession; title: string; fallback_title: string; native_title: string | null; manual_title: string | null; cwd: string | null; ended_at_unix_ms: number | null; open_missed: number; subtree_open_missed: number; missed: MissedWork[] };
export type SessionRecordSection = { session: WorkSession | null; loading: boolean; failure: string | null; record: SessionRecord | null };
export type SessionSearchStatus = "current" | "working" | "done" | "needs_you" | "archived" | "retained";
export type SessionSearchCursor = { at_unix_ms: number; device_id: string; agent: string; session_id: string };
export type SessionSearchOptions = { request_id?: string | null; device_id?: string | null; project_root?: string | null; agent?: string | null; since_unix_ms?: number | null; until_unix_ms?: number | null; status?: SessionSearchStatus | null; include_titles?: boolean; include_tasks?: boolean; cursor?: SessionSearchCursor | null };
export type SessionSearchMatch = { kind: "request" | "title" | "task"; text: string; at_unix_ms: number; request_offset: number | null };
export type SessionSearchHit = { session: WorkSession; title: string; matched: string; at_unix_ms: number; cwd: string | null; project_root?: string | null; status?: SessionSearchStatus; matches?: SessionSearchMatch[]; match_count?: number; request_count?: number };
export type SessionWorkSearch = { query: string; options?: SessionSearchOptions; loading: boolean; failure: string | null; results: SessionSearchHit[]; next_cursor?: SessionSearchCursor | null };

/** A durable conversation intent, never a current pane or delivery target. */
export type SleepingSession = {
  sleep_id: string;
  node_id: string;
  checkout_path: string;
  kind: string;
  identity_label: string;
  group: string;
  phase: "saving_close" | "saving_close_ready" | "closing" | "close_unknown" | "sleeping" | "saving_wake" | "creating" | "saving_start" | "starting" | "wake_unknown" | "failed";
  since_unix_ms: number;
  reason: string | null;
  wake_available: boolean;
  checking: boolean;
};

/** One archived root and the descendants archived with it (`ArchivedSessionSnapshot`, PRD session-archive). */
export type ArchivedSession = {
  id: string;
  device_id: string;
  project_root: string;
  project_label: string;
  checkout_path: string;
  phase: "archiving" | "close_failed" | "unknown" | "archived" | "deleting";
  /** Whether the row stands in `보관됨`; otherwise its root is live and the members show in its record (B24). */
  listed: boolean;
  archived_at_unix_ms: number;
  reason: string | null;
  /** The root carries the operator's ★ (PRD revisit-group B15). */
  starred?: boolean;
  /** The root first. */
  members: ArchivedMember[];
};
export type ArchivedMember = {
  session: WorkSession;
  agent_kind: string;
  title: string;
  depth: number;
  parent_session_id: string | null;
  last_line: string | null;
  pulls: { url: string; number: number; title: string; state: PrState }[];
  resume: "archived" | "queued" | "starting" | "failed" | "unknown" | "resumed";
  reason: string | null;
};

/** Retained letter and passive direct-input metadata with exact session proof. No bodies. */
export type OverviewGraphParty = { device_id: string; pane_id: string; session: string };
export type OverviewGraphLetter = { id: string; sender: OverviewGraphParty; recipient: OverviewGraphParty; kind: string; at_unix_ms: number };
export type OverviewGraphMetadata = {
  available: boolean;
  limit: number;
  retained_count: number;
  truncated: boolean;
  retention_ms: number;
  letters: OverviewGraphLetter[];
  direct_inputs?: { id: string; sender: OverviewGraphParty; recipient: OverviewGraphParty; at_unix_ms: number }[];
  sessions: { device_id: string; pane_id: string; session_digest: string; launch_model: string | null }[];
};

export type SnapshotRest = {
  overview_graph?: OverviewGraphMetadata;
  git_worktrees_loading?: boolean;
  navigator?: {
    agent_scope?: AgentScope;
    focused_workspace_id?: string | null;
    focused_checkout_id?: string | null;
    /** The focused checkout's root, which the Explorer reveals under. */
    root_path?: string | null;
    /** History's registered-folder scope; it may be narrower than root_path. */
    changes_root_path?: string | null;
    workspaces?: Workspace[];
    inactive_projects?: InactiveProjectGroup[];
    /** Each device's projects with no agent and no sleeping conversation (D-01); opened by the `no_agents/<device>` fold key. */
    no_agent_projects?: InactiveProjectGroup[];
    agents?: AgentRow[];
    hide_ai_checkouts?: HideAiCheckout[];
    sleeping_sessions?: SleepingSession[];
    /** Newest archived first (D-09). */
    archived_sessions?: ArchivedSession[];
    devices?: Device[];
    focused_device_id?: string | null;
  };
  connection?: { kind: string; state: string; target_id: string | null };
  task_operation?: TaskOperation | null;
  issue_work?: IssueWork;
  pr_work?: PrWork;
  repository_clone?: RepositoryClone | null;
  worktree_removal?: WorktreeRemoval | null;
  tab?: Tab;
  zoomed?: string | null;
  focused?: {
    pane_id?: string | null;
    /** The last operator `focus_pane` number the core applied, per page (`operatorFocus.ts`). */
    operator_focus?: { client_id: string; sequence: number }[];
  };
  pane_layouts?: PaneLayout[];
  terminal?: { headers?: Record<string, PaneHeader>; pane_id?: string | null; panes?: TerminalPane[]; input_requests?: InputRequest[] };
  ui_state?: {
    left_sidebar_visible?: boolean;
    device_rail_visible?: boolean;
    right_panel_visible?: boolean;
    right_panel_section?: string;
    workspace_registrations?: WorkspaceRegistration[];
    pane_text_scales?: Record<string, number>;
    editor_text_scale?: number;
    expanded_paths?: string[];
    /** Each SSH device's expanded Explorer folders; this machine's are `expanded_paths`. */
    device_expanded_paths?: Record<string, string[]>;
    /** Projects whose checkouts the sidebar folds. */
    collapsed_workspace_ids?: string[];
    /** Checkouts whose agent rows the Projects list opened; absence is closed, where line two names the agents. */
    expanded_checkout_ids?: string[];
    session_collapsed_checkout_ids?: string[];
    /** Agents whose children the sidebar tree opened; absence is folded. */
    expanded_agent_pane_ids?: string[];
    /** Agents whose children the sidebar's 태스크 list opened, kept apart from the 프로젝트 tree's (PRD sidebar-task-lens D-48). */
    sessions_expanded_agent_pane_ids?: string[];
    /** Which list the sidebar shows (`contracts/snapshot-wire-enums.json`: `sidebar_lens`, D-10); absent reads as projects. */
    sidebar_lens?: SidebarLens;
    /** How the 태스크 list groups (`task_view`, D-11); absent reads as status. */
    task_view?: TaskView;
    /** The lane's 보기 slot (D-50), one per window. */
    detail?: DetailSlot;
    selected_path?: string | null;
    selected_pane_id?: string | null;
    accent_hex?: string;
    /** `system`, `light` or `dark`; the page reads anything else as Dark. */
    theme?: string;
    /** Whether list rows and tabs draw the agent's logo (`contracts/snapshot-wire-enums.json`: `agent_logo_mode`); absent reads as hidden. */
    agent_logo_mode?: AgentLogoMode;
    /** `hide` or `neutral` (`contracts/snapshot-wire-enums.json`: `color_theme`); the page reads anything else as Hide. */
    color_theme?: string;
    /** Core-confirmed explicit choice; null follows this client's system language. */
    interface_language?: "en" | "ko" | "zh-CN" | "ja" | null;
    /** The first-run agent choice (`contracts/snapshot-wire-enums.json`: `agent_onboarding`): absent until this Mac's kit answered, `pending` while its kit record waits for the operator's answer. */
    agent_onboarding?: "pending" | "done" | null;
    /** The agents left on in that choice. */
    agent_onboarding_agents?: string[];
    /** The first-run wizard's progress (PRD first-run-onboarding D-07): absent on a Mac that never had one; another flow `version` is not this build's to show. */
    first_run?: FirstRun | null;
    first_run_choices?: FirstRunChoices;
    /** Settings > Notifications' Notify me when an agent needs me (D-21); the desktop app shows agent notifications only while it is on. */
    desktop_notifications?: boolean;
    product_analytics_enabled?: boolean;
    desktop_notifications_done?: boolean;
    usage?: UsagePreferences;
    /** The screens whose one-line introduction was shown (B43). */
    screen_intros_seen?: ScreenIntro[];
    font_size?: number;
    /** The sidebar's width in CSS pixels, 220 to 440; the core refuses anything else (PRD sidebar-typography D-09). */
    sidebar_width?: number;
    /** The desktop app's macOS pane chords, in the removed native app's format. */
    shortcut_bindings?: Record<string, string>;
    browser_shortcut_bindings?: Record<string, string>;
    /** Sleep idle agents after this many hours; null or absent is Never (PRD agent-sleep). */
    agent_sleep_after_hours?: number | null;
    /** Each local project's chosen issue source (`github` or `local`) by path; absent is the default. */
    project_issue_sources?: Record<string, string>;
    project_base_branches?: Record<string, string>;
    project_settings?: Record<string, ProjectSettings>;
    project_settings_writes?: Record<string, ProjectSettingsWrite & { device_id: string; root: string }>;
    issue_settings?: IssueSettings;
    agent_start?: AgentStartChoice;
    /** The checkouts last brought to the front, newest first, up to ten (PRD cmdk-recent). */
    recent_checkouts?: RecentCheckout[];
    /** The terminal panes the keyboard has been in, newest first, up to fifty: the Agent cycle's order (issue 301). */
    recent_pane_ids?: string[];
    [key: string]: unknown;
  };
  status?: {
    server_discovery?: { loading: boolean; failure: string | null };
    herdr?: HerdrStatus;
    remote?: RemoteStatus[];
    environment?: EnvironmentStatus[];
    agent_hooks?: AgentHooks;
    background_ai?: BackgroundAi;
    /** The Host entries Add device lists, read on an `ssh_hosts_list` event (PRD settings-cleanup D-19). */
    ssh_hosts?: SshHosts;
    herdr_profiles?: HerdrProfiles;
    diagnostics?: CoreDiagnostic[];
    async_operations?: AsyncOperation[];
    /** Bounded read-only hover text; never an attachment or a focus request. */
    pane_move_preview?: { request_id: string; tab_id: string; phase: "preparing" | "ready" | "failed"; panes: { pane_id: string; text: string }[]; message?: string | null } | null;
    tab_rename?: { request_id: string; tab_id: string; label: string; phase: "pending" | "succeeded" | "failed" } | null;
    pane_focus_request?: PaneFocusRequest | null;
    /** The answers to the latest `browser_open` events that named themselves, oldest first and bounded. */
    browser_opens?: import("./viewLayout").BrowserOpenReceipt[];
    /** The core's most recent failure; the shell logs its detail (B5). */
    last_error?: { kind: string; message: string; retryable: boolean; occurred_at: number; request_id?: string | null } | null;
  };
  recent_closed?: RecentClosed;
  explorer_operation?: ExplorerOperation | null;
  /** The front Workspace's layout and tools (S6 D-10); absent when no Workspace is in front. */
  workspace_view?: import("./workspace").WorkspaceView | null;
  /** All retained Browser Views, including those outside the front Workspace. */
  browser_views?: { device_id: string; path: string; view_id: string; area_id: string }[];
  /** Positive connected Workspace area authority from the core, including empty areas; incarnation changes on revoke/regrant. */
  browser_scopes?: { device_id: string; path: string; area_id: string; incarnation: number }[];
};

/**
 * The SSH device the operator selected, or null while this machine is the
 * context. With one selected, the local tabs are not what the operator is
 * looking at, so the shell neither draws them nor sends them pane commands
 * (PRD S5 B19), the way the native shell's remote context works.
 */
/**
 * The core's own node id: the row of kind `local`, which the core names by
 * its machine's id. Before the snapshot names it the id is empty, which an
 * event carries as no `device_id`, and the core reads that as its own node.
 */
export function localDeviceId(rest: SnapshotRest | null): string {
  return rest?.navigator?.devices?.find((device) => device.kind !== "remote")?.id ?? "";
}

/**
 * Every device row named by the device it runs on. The core publishes a
 * device's agents inside that device's status, where the row does not say
 * whose it is, but a session's ledger row is kept under the device id, so
 * every reader that names a row's session (a pane head, a task row, a reveal)
 * has to find the id on the row. This is the one place that sets it; rows
 * already carrying it, and statuses with no agents, are returned as they are
 * so `share` keeps their identity.
 */
export function stampDeviceAgents(rest: SnapshotRest): SnapshotRest {
  const remote = rest.status?.remote;
  if (!remote?.some((status) => status.session?.agents?.some((agent) => agent.device_id !== status.target_id))) return rest;
  return {
    ...rest,
    status: {
      ...rest.status!,
      remote: remote.map((status) => {
        const session = status.session;
        if (!session?.agents?.some((agent) => agent.device_id !== status.target_id)) return status;
        return { ...status, session: { ...session, agents: session.agents.map((agent) => (agent.device_id === status.target_id ? agent : { ...agent, device_id: status.target_id })) } };
      }),
    },
  };
}

export function focusedRemoteDevice(rest: SnapshotRest | null): Device | null {
  const id = rest?.navigator?.focused_device_id;
  if (!id) return null;
  return rest?.navigator?.devices?.find((device) => device.id === id && device.kind === "remote") ?? null;
}

export function focusedCheckout(rest: SnapshotRest | null): Checkout | null {
  const id = rest?.navigator?.focused_checkout_id;
  if (!id) return null;
  for (const workspace of rest?.navigator?.workspaces ?? []) {
    const checkout = workspace.checkouts.find((c) => c.id === id);
    if (checkout) return checkout;
  }
  return null;
}

/**
 * Every project the core's catalog carries: this machine's and registered ones
 * in the navigator, then each device's Herdr session. The core looks a
 * checkout up in the same two places (`catalog_checkout`); device ids are
 * scoped, so one id never names checkouts on two machines.
 */
export function catalogWorkspaces(rest: SnapshotRest | null): Workspace[] {
  return catalogOf(rest?.navigator?.workspaces, rest?.status?.remote);
}

/** `catalogWorkspaces` from the two fields it reads, for a component that subscribes to those alone. */
export function catalogOf(workspaces: Workspace[] | undefined, remote: RemoteStatus[] | undefined): Workspace[] {
  return [...(workspaces ?? []), ...(remote ?? []).flatMap((row) => row.session?.workspaces ?? [])];
}

export function checkoutById(rest: SnapshotRest | null, id: string): Checkout | null {
  for (const workspace of catalogWorkspaces(rest)) {
    const checkout = workspace.checkouts.find((c) => c.id === id);
    if (checkout) return checkout;
  }
  return null;
}

/** The device a catalog checkout lives on; the core's own node for one it does not list. */
export function deviceOfCheckout(rest: SnapshotRest | null, id: string): string {
  return catalogWorkspaces(rest).find((workspace) => workspace.checkouts.some((c) => c.id === id))?.device_id ?? localDeviceId(rest);
}

/**
 * The checkout the operator is looking at: this machine's focus while it is
 * the selected device, otherwise that device's own Herdr focus, which the core
 * follows (`front_checkout`).
 */
export function frontCheckout(rest: SnapshotRest | null): Checkout | null {
  const device = focusedRemoteDevice(rest);
  if (!device) return focusedCheckout(rest);
  const session = rest?.status?.remote?.find((remote) => remote.target_id === device.id)?.session;
  const id = session?.focused_checkout_id;
  if (!id) return null;
  return session?.workspaces.flatMap((workspace) => workspace.checkouts).find((c) => c.id === id) ?? null;
}

export function visibleTab(checkout: Checkout | null): Tab | null {
  if (!checkout?.active_tab_id) return null;
  return checkout.tabs.find((tab) => tab.id === checkout.active_tab_id) ?? null;
}

export function layoutForTab(rest: SnapshotRest | null, tabId: string | null): PaneLayout | null {
  if (!tabId) return null;
  return rest?.pane_layouts?.find((layout) => layout.tab_id === tabId) ?? null;
}

/**
 * The core's editor section when the active View area shows an open
 * document, else null: a chord that could mean the document or the pane
 * (⌘F, text size) reads null as "the terminal owns it".
 */
export function editorFor(editor: EditorSnapshot | null): EditorSnapshot | null {
  return editor?.active_tab_id ? editor : null;
}

/** The editor tab (document) with this id, or null once the core closed it. */
export function editorTabFor(editor: EditorSnapshot | null, tabId: string | null): EditorTabSnapshot | null {
  if (!tabId) return null;
  return editor?.tabs.find((tab) => tab.id === tabId) ?? null;
}

const NO_PATHS: string[] = [];

/**
 * What the Explorer draws: the selected device, the checkout in front on it,
 * and that device's expanded folders. A path is only ever read against its
 * own device, so the same path on two machines never shares a row (S5.5 B2).
 */
export function explorerContext(rest: SnapshotRest | null): { device: string; checkout: Checkout | null; expanded: string[] } {
  const node = localDeviceId(rest);
  const device = focusedRemoteDevice(rest)?.id ?? node;
  const state = rest?.ui_state;
  return {
    device,
    checkout: frontCheckout(rest),
    expanded: (device === node ? state?.expanded_paths : state?.device_expanded_paths?.[device]) ?? NO_PATHS,
  };
}

/** The changes of the checkout a listing belongs to, or null when they are another checkout's. */
export function changesFor(changes: ChangesSnapshot | null, rootPath: string | null): ChangesSnapshot | null {
  if (!changes || !rootPath || changes.root_path !== rootPath) return null;
  return changes;
}

export function paneIds(node: LayoutNode): string[] {
  if (node.type === "pane") return [node.pane_id];
  return [...paneIds(node.first), ...paneIds(node.second)];
}

/** The pane a divider between `first` and `second` names in `resize_pane`: the last pane of the first subtree. */
export function dividerPaneId(first: LayoutNode): string {
  const ids = paneIds(first);
  return ids[ids.length - 1] ?? "";
}

