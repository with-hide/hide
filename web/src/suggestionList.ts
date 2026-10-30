// The 제안 tab's reading of the core's rows (PRD hide-ai-suggestions D-07,
// D-37, B10-B17). The core decides every group, state, hold and count; this
// module only sorts what it sent into the three groups and names each row's
// mark and line, so the components draw them without deciding anything.
import type { TFunction } from "i18next";
import type { MessageKey } from "./i18n/catalogs";
import type { GlobalInstructionUnusable, ProjectSuggestions, SuggestionBranch, SuggestionLocalFailure, SuggestionRow, SuggestionState } from "./snapshot";

export type SuggestionGroups = { choose: SuggestionRow[]; applying: SuggestionRow[]; applied: SuggestionRow[] };

/** The rows by group, each in the core's order. */
export function suggestionGroups(rows: readonly SuggestionRow[]): SuggestionGroups {
  const groups: SuggestionGroups = { choose: [], applying: [], applied: [] };
  for (const row of rows) groups[row.group].push(row);
  return groups;
}

/**
 * A row's mark (D-37): the green open circle for a new suggestion, the yellow
 * dot for the operator's turn, the blue turning ring while Hide AI works; the
 * rows under 반영됨 carry none, their line says it.
 */
export type SuggestionMark = "new" | "my_turn" | "working" | null;

export function suggestionMark(state: SuggestionState): SuggestionMark {
  switch (state) {
    case "new": return "new";
    case "my_turn": return "my_turn";
    case "hide_ai_working": return "working";
    case "watching":
    case "effective":
    case "applied": return null;
  }
}

/**
 * A row's title: a Memory batch's `정정 N개 문서에 반영`, a document fix's
 * result (what its press changes), else the discomfort in a person's words,
 * else the signal and tag the grouping wrote it from.
 */
export function suggestionTitle(t: TFunction<"translation">, row: Pick<SuggestionRow, "id" | "discomfort" | "basis" | "memory_cards" | "plan">): string {
  if (row.memory_cards && row.memory_cards.length > 0) return t("suggestions.memoryBatch", { count: row.memory_cards.length });
  if (row.plan) return row.plan.title;
  return reasonText(t, row) ?? row.id;
}

/** Why a row is there: the discomfort in a person's words, else the signal and tag the grouping wrote it from. */
function reasonText(t: TFunction<"translation">, row: Pick<SuggestionRow, "discomfort" | "basis">): string | null {
  if (row.discomfort) return row.discomfort;
  if (row.basis) return t(`suggestions.basis.${row.basis.signal}` as MessageKey, { tag: row.basis.tag });
  return null;
}

/** The line under a document fix's title: why it is there, since its title says what it changes; null for a row whose title already says why. */
export function suggestionReason(t: TFunction<"translation">, row: Pick<SuggestionRow, "discomfort" | "basis" | "plan">): string | null {
  return row.plan ? reasonText(t, row) : null;
}

/** A repository path's last name, which a document fix's button names. */
export function fileName(path: string): string {
  return path.split("/").filter(Boolean).at(-1) ?? path;
}

/**
 * Whether a row is a suggestion not pressed yet, which offers its fix; every
 * other row under 고를 것 is an attempt that waits for the operator's choice.
 */
export function offered(row: Pick<SuggestionRow, "state">): boolean {
  return row.state === "new";
}

/**
 * How a status line reads: the words, their tone, and what the operator does
 * from it: `다시 시도` with `넘어가기` on a failed attempt, or a ready branch's
 * decision (PRD hide-ai-branch-first-apply B3-B9), where `retry` is the
 * action a refused merge offers again; `files` are the ones a sync found
 * unsaved.
 */
export type AttemptLine = { text: string; tone: "muted" | "warning" | "destructive"; retry: boolean; branch?: BranchChoice; files?: string[]; worded?: true };

/**
 * A ready branch's buttons: the action that answers its last refusal, if any
 * (B7, B8): `pr` after a push GitHub did not take, `sync` after a push that
 * found the local base apart from GitHub's.
 */
export type BranchChoice = { again: "merge" | "rebase" | "discard" | "pr" | "sync" | null };

/** The line of a row being applied (B2-B12, B14): what the attempt is doing, in words; null before one started. */
export function attemptLine(t: TFunction<"translation">, row: Pick<SuggestionRow, "outcome" | "attempt">): AttemptLine | null {
  const attempt = row.attempt;
  if (!attempt) return null;
  const issue = row.outcome === "issue";
  const base = attempt.branch?.base ?? "main";
  switch (attempt.phase) {
    case "starting":
    case "working": {
      if (issue) return { text: t("suggestions.attempt.writingIssue"), tone: "muted", retry: false };
      const key = attempt.purpose === "branch" ? "suggestions.attempt.fixing" : attempt.purpose === "rebase" ? "suggestions.attempt.rebasing" : "suggestions.attempt.writingPr";
      return { text: t(key, { base }), tone: "muted", retry: false };
    }
    case "ready": {
      const code = attempt.failure;
      if (!code) return { text: t("suggestions.branch.view"), tone: "muted", retry: false, branch: { again: null } };
      const refusal = REFUSALS[code];
      const text = refusal ? refusalWords(t, refusal, attempt.branch ?? null, base) : t("suggestions.failure.lead.merge");
      const files = code === "sync_dirty" ? attempt.branch?.base_unsaved ?? [] : [];
      return { text, tone: "warning", retry: false, branch: { again: refusal?.again ?? "merge" }, ...(files.length > 0 ? { files } : {}) };
    }
    case "merging": return { text: t("suggestions.attempt.merging"), tone: "muted", retry: false };
    case "syncing": return { text: t("suggestions.attempt.syncing", { base }), tone: "muted", retry: false };
    case "pr_open": return { text: attempt.pr_number === null ? t("suggestions.attempt.writingPr") : t("suggestions.attempt.prReview", { number: String(attempt.pr_number) }), tone: "warning", retry: false };
    case "ci_failed": return { text: t("suggestions.attempt.ciFailed"), tone: "destructive", retry: true };
    case "failed": {
      // A start Memory refused reads why in these words; the core's sentence under it is for a code this build has no words for.
      const refusal = attempt.failure ? START_REFUSAL_KEYS[attempt.failure] : undefined;
      if (refusal) return { text: `${t("suggestions.failure.not_started")} · ${t(refusal, { base })}`, tone: "warning", retry: true, worded: true };
      // A known reason says what went wrong in place of the generic ending; an unknown code reads as the ending.
      const reason = attempt.failure ? failureWords(t, attempt.failure, base) : null;
      const ending = issue ? "suggestions.attempt.failedIssue" : attempt.purpose === "branch" || attempt.purpose === "rebase" ? "suggestions.failure.no_change" : "suggestions.attempt.failed";
      return { text: reason ?? t(ending), tone: "warning", retry: true };
    }
    case "issue_open": return { text: t("suggestions.attempt.issueOpen"), tone: "muted", retry: false };
    case "merged": return { text: t("suggestions.attempt.merged"), tone: "muted", retry: false };
  }
}

/** The failure codes of an attempt that ended, which the catalogs word; a code of another build reads as the generic ending, and its detail is in the log. */
const FAILURE_KEYS: Record<string, MessageKey> = {
  agent_not_started: "suggestions.failure.agent_not_started",
  no_pr: "suggestions.failure.no_pr",
  shell_starting: "suggestions.failure.shell_starting",
  start_failed: "suggestions.failure.start_failed",
  not_started: "suggestions.failure.not_started",
  card_not_processed: "suggestions.failure.card_not_processed",
  unbuilt: "suggestions.failure.unbuilt",
  issue_failed: "suggestions.failure.issue_failed",
  no_change: "suggestions.failure.no_change",
};

/** Why Memory refused to start a press, by the code the core put on the row (`herdr-core/src/runtime/suggestion_memory.rs`, `Refusal`). */
const START_REFUSAL_KEYS: Record<string, MessageKey> = {
  branches_full: "suggestions.refusal.branches_full",
  not_git: "suggestions.refusal.not_git",
  promotion_unavailable: "suggestions.refusal.promotion_unavailable",
  action_busy: "suggestions.refusal.action_busy",
  presses_full: "suggestions.refusal.presses_full",
  store_unavailable: "suggestions.refusal.store_unavailable",
  project_unavailable: "suggestions.refusal.project_unavailable",
  project_unreachable: "suggestions.refusal.project_unreachable",
};

function failureWords(t: TFunction<"translation">, code: string, base: string): string | null {
  const key = FAILURE_KEYS[code];
  return key ? t(key, { base }) : null;
}

/**
 * A ready branch's refusal: what did not happen (`합치지 못했어요`,
 * `올리지 못했어요`, `맞추지 못했어요`, `버리지 못했어요`), why in one line
 * after it, and the button that answers it. A code this build does not know
 * reads `합치지 못했어요` with `다시 시도`; its reason is in the log.
 */
type Refusal = { lead: MessageKey; reason: MessageKey | "apart" | null; again: BranchChoice["again"] };

const merge = (reason: MessageKey, again: BranchChoice["again"] = "merge"): Refusal => ({ lead: "suggestions.failure.lead.merge", reason, again });
const push = (reason: MessageKey | "apart", again: BranchChoice["again"] = "pr"): Refusal => ({ lead: "suggestions.failure.lead.push", reason, again });
const sync = (reason: MessageKey): Refusal => ({ lead: "suggestions.failure.lead.sync", reason, again: "sync" });

const REFUSALS: Record<string, Refusal> = {
  merge_dirty: merge("suggestions.failure.merge_dirty"),
  merge_in_progress: merge("suggestions.failure.in_progress"),
  merge_conflict: merge("suggestions.failure.merge_conflict", "rebase"),
  merge_moved: merge("suggestions.failure.merge_moved"),
  merge_branch_moved: merge("suggestions.failure.merge_branch_moved"),
  merge_sign_failed: merge("suggestions.failure.sign_failed"),
  merge_timed_out: merge("suggestions.failure.timed_out"),
  merge_worktree_missing: merge("suggestions.failure.worktree_missing"),
  merge_unreachable: merge("suggestions.failure.unreachable"),
  merge_unknown: merge("suggestions.failure.unknown"),
  merge_failed: merge("suggestions.failure.git_failed"),
  discard_failed: { lead: "suggestions.failure.lead.discard", reason: null, again: "discard" },
  push_refused: push("suggestions.failure.push_refused"),
  push_failed: push("suggestions.failure.push_failed"),
  push_kept: push("suggestions.failure.push_kept"),
  push_base_ahead: push("apart", "sync"),
  push_base_diverged: push("apart", "sync"),
  push_fetch_failed: push("suggestions.failure.push_fetch_failed", "merge"),
  push_no_upstream: push("suggestions.failure.push_no_upstream"),
  sync_dirty: sync("suggestions.failure.sync_dirty"),
  sync_in_progress: sync("suggestions.failure.in_progress"),
  sync_moved: sync("suggestions.failure.sync_moved"),
  sync_fetch_failed: sync("suggestions.failure.push_fetch_failed"),
  sync_no_upstream: sync("suggestions.failure.push_no_upstream"),
  sync_timed_out: sync("suggestions.failure.timed_out"),
  sync_worktree_missing: sync("suggestions.failure.worktree_missing"),
  sync_unreachable: sync("suggestions.failure.unreachable"),
  sync_unknown: sync("suggestions.failure.unknown"),
  sync_failed: sync("suggestions.failure.git_failed"),
};

function refusalWords(t: TFunction<"translation">, refusal: Refusal, branch: SuggestionBranch | null, base: string): string {
  const lead = t(refusal.lead);
  if (refusal.reason === null) return lead;
  // How many commits the local base has of its own, once a push counted them.
  const reason = refusal.reason === "apart"
    ? branch?.base_unpushed ? t("suggestions.failure.push_base_unpushed", { base, count: branch.base_unpushed }) : t("suggestions.failure.push_base_apart", { base })
    : t(refusal.reason, { base });
  return `${lead} · ${reason}`;
}

/** The fold's line: how many rows are under 반영됨 and how many of them worked (B15). */
export function appliedSummary(applied: readonly SuggestionRow[]): { count: number; effective: number } {
  return { count: applied.length, effective: applied.filter((row) => row.state === "effective").length };
}

/** The Memory tab's own number: the rail's, less the 제안 tab's (D-01). */
export function memoryTabCount(project: Pick<ProjectSuggestions, "counts">): number {
  return Math.max(0, project.counts.rail - project.counts.suggestions);
}

/**
 * Whether a row offers its apply button: a new suggestion, `문서 고치기`
 * with or without GitHub (PRD hide-ai-branch-first-apply B1), an issue only
 * with it, where `GitHub 연결 필요` stands instead (B10, B18), and neither
 * while five branches wait for a decision (B28). A global instruction
 * row needs no GitHub (PRD memory-self-improve B8): it offers its press
 * while the core gave a preview to send back, and shows it held while none
 * of its evidence could be read (B7) or the line has no room (B10).
 */
export function applyOffer(row: Pick<SuggestionRow, "state" | "hold" | "outcome" | "local" | "evidence_unconfirmed">): "pr" | "issue" | "local" | "local_held" | "github_needed" | "branches_full" | null {
  if (!offered(row)) return null;
  if (row.outcome === "local") {
    if (!row.local || row.local.present) return null;
    return row.local.preview !== null && !row.evidence_unconfirmed ? "local" : "local_held";
  }
  if (row.hold === "github_needed") return "github_needed";
  if (row.hold === "branches_full") return "branches_full";
  return row.outcome;
}

/** A global instruction row's line under its title, its tone, and whether it offers `파일 열기` (B5, B9, B10, B11, B29). */
export type LocalLine = { text: string; tone: "muted" | "warning" | "success"; openFile: boolean };

const LOCAL_FAILURE_KEYS: Record<SuggestionLocalFailure, MessageKey> = {
  no_room: "suggestions.local.failure.no_room",
  too_large: "suggestions.local.failure.too_large",
  unreadable: "suggestions.local.failure.unreadable",
  changed: "suggestions.local.failure.changed",
  edited: "suggestions.local.failure.edited",
  write_failed: "suggestions.local.failure.write_failed",
  evidence_unconfirmed: "suggestions.local.failure.evidence_unconfirmed",
  bad_line: "suggestions.local.failure.bad_line",
};

/** The failures the operator settles in the file itself. */
const OPENS_FILE: ReadonlySet<SuggestionLocalFailure> = new Set(["no_room", "too_large", "unreadable", "edited"]);

export function localLine(t: TFunction<"translation">, row: Pick<SuggestionRow, "group" | "local" | "evidence_unconfirmed">, unusable: GlobalInstructionUnusable | null): LocalLine | null {
  const local = row.local;
  if (!local) return null;
  // The open evidence decides before the last press: none of its words could be read now (B7).
  const failure: SuggestionLocalFailure | null = row.group === "choose" && row.evidence_unconfirmed ? "evidence_unconfirmed" : local.failure;
  if (failure) return { text: t(LOCAL_FAILURE_KEYS[failure]), tone: "warning", openFile: OPENS_FILE.has(failure) };
  // No preview and no reason: the file cannot be read as it is, which the tab's own line says once.
  if (row.group === "choose" && local.preview === null && !local.present && unusable) return null;
  // Said again with the line in the file: the operator rewords it there, or skips it.
  if (row.group === "choose" && local.recurred) return { text: t("suggestions.local.recurred"), tone: "warning", openFile: true };
  if (local.present) return { text: t(row.group === "choose" ? "suggestions.local.present" : "suggestions.local.applied"), tone: row.group === "choose" ? "muted" : "success", openFile: false };
  return null;
}
