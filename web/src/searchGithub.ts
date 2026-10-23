// What ⌘K asks GitHub and how it says so (PRD cmdk-navigation B8-B10, B16-B19).
// Two readings meet here: the one read of the project in front, which ⌘K asks
// for once and which the core answers on the project's own checkouts, and the
// explicit search the `Search GitHub for "…"` row starts. Typing never calls
// GitHub; both are pure decisions over the snapshot so they test without a
// browser.

import type { TFunction } from "i18next";
import { readsHere } from "./devices";
import type { InterfaceLanguage } from "./i18n/locale";
import { ageWords } from "./overviewLens";
import { catalogWorkspaces, type Checkout, type GithubSearch, type SnapshotRest, type Workspace } from "./snapshot";

/** The age words the detail and the failure tooltip use, from the shared elapsed-time words. */
function readAge(at: number | null | undefined, now: number, t: TFunction<"translation">, language: InterfaceLanguage): string | null {
  return at == null ? null : ageWords(language, now - at, t);
}

function githubStatus(workspace: Workspace) {
  return workspace.checkouts.find((checkout) => checkout.github)?.github ?? null;
}

/** Whether the core has answered this project's pull requests: a success on record, or the answer that it has no GitHub remote. */
function answered(workspace: Workspace): boolean {
  const status = githubStatus(workspace);
  return status?.last_success_at_unix_ms != null || status?.failure_category === "no_github_remote";
}

/**
 * The project ⌘K reads when it opens (B8, D-12): the project in front whose
 * GitHub the core reads (its own, or a node's that dials it, `readsHere`),
 * once per app run, when no screen has asked for it. A project the core has
 * answered, one already asked for here, and a dialed device's project are
 * left alone.
 * `source.reading` alone is not "in flight": a project nobody asked for
 * reports it forever (tasks.rs), so it is never read as a request.
 */
export function projectToRead(rest: SnapshotRest | null, frontCheckout: Checkout | null, asked: ReadonlySet<string>): Workspace | null {
  if (!rest || !frontCheckout) return null;
  const workspace = catalogWorkspaces(rest).find((row) => row.id === frontCheckout.workspace_id);
  if (!workspace || workspace.is_home || !workspace.is_git || !readsHere(workspace, rest.navigator?.devices)) return null;
  if (asked.has(workspace.id) || answered(workspace)) return null;
  return workspace;
}

/** How a project's read stands for the rows that name its checkouts. */
export type ProjectRead = { state: "reading" } | { state: "failed"; tooltip: string } | null;

/**
 * Reading while ⌘K asked and the core has not answered, failed once the core
 * says so, with the last value's age (B9); nothing otherwise, so a project
 * nobody asked for never shows a progress mark.
 */
export function projectRead(workspace: Workspace, asked: ReadonlySet<string>, now: number, t: TFunction<"translation">, language: InterfaceLanguage): ProjectRead {
  const status = githubStatus(workspace);
  const failure = workspace.tasks?.source?.failure ?? null;
  const failed = status?.failure_category !== "no_github_remote" && (status?.stale === true || (status?.failure_category != null && !status.available) || failure !== null);
  if (failed) {
    const age = readAge(status?.last_success_at_unix_ms ?? workspace.tasks?.source?.last_read_at_unix_ms, now, t, language);
    return { state: "failed", tooltip: age ? t("search.readFailedWithAge", { age }) : t("search.readFailedNoValue") };
  }
  if (asked.has(workspace.id) && !answered(workspace)) return { state: "reading" };
  return null;
}

/** The `Read 3 min. ago` a pull request's or issue's detail carries (B10): when its project was last read. */
export function lastReadWords(workspace: Workspace, now: number, t: TFunction<"translation">, language: InterfaceLanguage): string | null {
  const at = githubStatus(workspace)?.last_success_at_unix_ms ?? workspace.tasks?.source?.last_read_at_unix_ms;
  const age = readAge(at, now, t, language);
  return age ? t("search.lastRead", { age }) : null;
}

/** Whether the core has a project a GitHub search could look in (B16): its own or a node's, by the source it publishes. */
export function hasGithubProject(rest: SnapshotRest | null): boolean {
  return catalogWorkspaces(rest).some((workspace) => !workspace.is_home && workspace.is_git && workspace.tasks?.source?.kind === "github");
}

/** The state of the `Search GitHub for "…"` row. */
export type GithubRow = { state: "idle" | "working" | "failed" | "none"; label: string };

/**
 * The row's words for the answer to this query: the search itself while none
 * ran or one is running, a retry line when it failed (B18), a "none on GitHub
 * either" line once it answered with nothing, and says so when the core
 * searched only some of the repositories (its `message`).
 */
export function githubRow(query: string, answer: GithubSearch | null, t: TFunction<"translation">): GithubRow {
  const label = t("search.githubQuery", { query });
  if (!answer) return { state: "idle", label };
  if (answer.phase === "working") return { state: "working", label };
  if (answer.phase === "failed") return { state: "failed", label: t(answer.message === "project_settings_repository_changed" ? "projectSettings.repositoryChanged" : "search.githubFailed") };
  // A search the core cut short does not claim GitHub has nothing.
  if (answer.results.length > 0) return { state: "idle", label };
  return { state: "none", label: answer.message ? t("search.githubPartialNone") : t("search.githubNone") };
}

/** The core's answer for what this palette asked: only the request it sent, and only while the query is still the one it searched (B19). */
export function ownAnswer(search: GithubSearch | null | undefined, asked: { requestId: string; query: string } | null, query: string): GithubSearch | null {
  if (!search || !asked || search.request_id !== asked.requestId || asked.query !== query) return null;
  return search;
}

/** Whether choosing the row starts a search: not while the same query is already running (B19). */
export function startsSearch(row: GithubRow): boolean {
  return row.state !== "working" && row.state !== "none";
}
