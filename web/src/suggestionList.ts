// The 제안 tab's reading of the core's rows (PRD hide-ai-suggestions D-07,
// D-37, B10-B17). The core decides every group, state, hold and count; this
// module only sorts what it sent into the three groups and names each row's
// mark and line, so the components draw them without deciding anything.
import type { TFunction } from "i18next";
import type { MessageKey } from "./i18n/catalogs";
import type { ProjectSuggestions, SuggestionRow, SuggestionState } from "./snapshot";

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

/** A row's title: its discomfort in a person's words, else the signal and tag the grouping wrote it from. */
export function suggestionTitle(t: TFunction<"translation">, row: Pick<SuggestionRow, "id" | "discomfort" | "basis">): string {
  if (row.discomfort) return row.discomfort;
  if (row.basis) return t(`suggestions.basis.${row.basis.signal}` as MessageKey, { tag: row.basis.tag });
  return row.id;
}

/** How a status line reads: the words, their tone, and whether the operator acts on it (`다시 시도` and `안 함`). */
export type AttemptLine = { text: string; tone: "muted" | "warning" | "destructive"; retry: boolean };

/** The line of a row being applied (B11, B12, B14): what the attempt is doing, in words; null before one started. */
export function attemptLine(t: TFunction<"translation">, row: Pick<SuggestionRow, "outcome" | "attempt">): AttemptLine | null {
  const attempt = row.attempt;
  if (!attempt) return null;
  const issue = row.outcome === "issue";
  switch (attempt.phase) {
    case "starting":
    case "working": return { text: t(issue ? "suggestions.attempt.writingIssue" : "suggestions.attempt.writingPr"), tone: "muted", retry: false };
    case "pr_open": return { text: attempt.pr_number === null ? t("suggestions.attempt.writingPr") : t("suggestions.attempt.prReview", { number: String(attempt.pr_number) }), tone: "warning", retry: false };
    case "ci_failed": return { text: t("suggestions.attempt.ciFailed"), tone: "destructive", retry: true };
    case "failed": {
      // A known reason says what went wrong in place of the generic ending; an unknown code reads as the ending.
      const reason = attempt.failure ? failureWords(t, attempt.failure) : null;
      return { text: reason ?? t(issue ? "suggestions.attempt.failedIssue" : "suggestions.attempt.failed"), tone: "warning", retry: true };
    }
    case "issue_open": return { text: t("suggestions.attempt.issueOpen"), tone: "muted", retry: false };
    case "merged": return { text: t("suggestions.attempt.merged"), tone: "muted", retry: false };
  }
}

/** The failure codes the catalogs word; a code of another build reads as the generic ending, and its detail is in the log. */
const FAILURE_KEYS: Record<string, MessageKey> = {
  agent_not_started: "suggestions.failure.agent_not_started",
  no_pr: "suggestions.failure.no_pr",
  shell_starting: "suggestions.failure.shell_starting",
  start_failed: "suggestions.failure.start_failed",
  card_not_processed: "suggestions.failure.card_not_processed",
  unbuilt: "suggestions.failure.unbuilt",
  issue_failed: "suggestions.failure.issue_failed",
};

function failureWords(t: TFunction<"translation">, code: string): string | null {
  const key = FAILURE_KEYS[code];
  return key ? t(key) : null;
}

/** The fold's line: how many rows are under 반영됨 and how many of them worked (B15). */
export function appliedSummary(applied: readonly SuggestionRow[]): { count: number; effective: number } {
  return { count: applied.length, effective: applied.filter((row) => row.state === "effective").length };
}

/** The Memory tab's own number: the rail's, less the 제안 tab's (D-01). */
export function memoryTabCount(project: Pick<ProjectSuggestions, "counts">): number {
  return Math.max(0, project.counts.rail - project.counts.suggestions);
}

/** Whether a row offers its apply button: a new suggestion, unless GitHub is needed, where that line stands instead (B10, B18). */
export function applyOffer(row: Pick<SuggestionRow, "group" | "hold" | "outcome">): "pr" | "issue" | "github_needed" | null {
  if (row.group !== "choose") return null;
  if (row.hold === "github_needed") return "github_needed";
  return row.outcome;
}
