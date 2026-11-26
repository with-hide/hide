// ⌘P and ⌘K while Desk is in front (PRD desk-2-ui D-23, D-24, B37-B39): where
// ⌘P searches, how an all-projects answer is put in order, and what a ⌘K
// pick opens on Desk's desk. The palettes themselves are the app's own;
// these are the decisions Desk adds, kept as functions of values so they are
// testable without a browser.

import type { DeskItem } from "./desk";
import type { DeskOpen } from "./deskDesk";
import { groupEntries, type SearchEntry, type SearchSection } from "./search";
import { catalogWorkspaces, checkoutAtRoot, checkoutOfPane, type SnapshotRest } from "./snapshot";

/** A file a ⌘P answer names, with the checkout it is in. */
export type DeskFileHit = { device_id: string; root_path: string; path: string; relative_path: string };

/** Where ⌘P searches on Desk: the checkout of the session open beside Desk, else every project at once (B37, B38). */
export type DeskFileScope = { kind: "checkout"; device: string; root: string } | { kind: "all" };

/**
 * A session whose checkout is not listed (its pane went away while its
 * panel was open) is no session to search, so ⌘P searches every project.
 */
export function deskFileScope(rest: SnapshotRest | null, session: string | null): DeskFileScope {
  if (session === null) return { kind: "all" };
  const found = checkoutOfPane(rest, session);
  return found ? { kind: "checkout", device: found.device, root: found.checkout.path } : { kind: "all" };
}

/**
 * The project Desk named last (D-23): the newest conversation line that
 * names a project, or a session whose project it is. A project is named in
 * the `@` menu by its primary checkout's path, a session by its pane.
 */
export function lastNamedWorkspace(rest: SnapshotRest | null, items: readonly DeskItem[]): string | null {
  const projects = catalogWorkspaces(rest);
  const ofProject = (id: string) => projects.find((project) => project.id === id || project.path === id || project.checkouts.some((checkout) => checkout.path === id))?.id ?? null;
  const ofPane = (pane: string) => checkoutOfPane(rest, pane)?.workspace ?? null;
  for (const item of [...items].reverse()) {
    if (item.kind === "user") {
      for (const mention of [...item.mentions].reverse()) {
        const found = mention.kind === "project" ? ofProject(mention.id) : ofPane(mention.id);
        if (found) return found;
      }
    } else if (item.kind === "reply") {
      for (const chip of [...item.mentions].reverse()) {
        const found = ofPane(chip.pane_id);
        if (found) return found;
      }
    }
  }
  return null;
}

/** The place the core is asked to put first: the project's primary checkout, which an all-projects search reads (`file_index_all`'s `prefer`). */
export function preferredRoot(rest: SnapshotRest | null, workspaceId: string | null): { device_id: string; root_path: string } | null {
  if (workspaceId === null) return null;
  const found = catalogWorkspaces(rest).find((project) => project.id === workspaceId);
  return found ? { device_id: found.device_id, root_path: found.path } : null;
}

/** One all-projects result as its row draws it: the muted `project ›`, the file's name and its folder. */
export type ProjectFileRow = { hit: DeskFileHit; project: string; name: string; folder: string };

/**
 * The answer in the order the list shows (B38): the named project's files
 * first, then the others, each keeping the rank hided gave it. A file of a
 * checkout that is no longer listed is left out, since nothing could open it.
 */
export function projectFileRows(rest: SnapshotRest | null, files: readonly DeskFileHit[], named: string | null): { named: ProjectFileRow[]; other: ProjectFileRow[] } {
  const projects = catalogWorkspaces(rest);
  const first: ProjectFileRow[] = [];
  const other: ProjectFileRow[] = [];
  for (const hit of files) {
    const place = checkoutAtRoot(rest, hit.device_id, hit.root_path);
    const project = place ? projects.find((one) => one.id === place.workspace) : null;
    if (!place || !project) continue;
    const parts = hit.relative_path.split("/");
    const row = { hit, project: project.label, name: parts.at(-1) ?? hit.relative_path, folder: parts.slice(0, -1).join("/") };
    (project.id === named ? first : other).push(row);
  }
  return { named: first, other };
}

/** The card ⌘K opens on Desk's desk for a PR or a mission row (PRD desk-own-desk B39), null for any other kind of row. */
export function deskTabOf(entry: SearchEntry): DeskOpen | null {
  if (entry.kind === "mission" && entry.missionId) return { kind: "card", card: { kind: "mission", mission: entry.missionId } };
  if (entry.kind === "pr" && !entry.external && entry.pr && entry.workspaceId) return { kind: "card", card: { kind: "pull_request", workspace_id: entry.workspaceId, number: entry.pr.number } };
  return null;
}

/** How many rows of each kind Desk's empty ⌘K lists; typing finds the rest. */
export const DESK_SEARCH_SHOWN = 8;

/**
 * What Desk's ⌘K lists before anything is typed (B39): the sessions, the pull
 * requests and the missions, each group in the snapshot's order and cut to
 * `DESK_SEARCH_SHOWN` rows.
 */
export function deskSections(entries: readonly SearchEntry[]): SearchSection[] {
  const kept = entries.filter((entry) => entry.kind === "agent" || (entry.kind === "pr" && !entry.external) || entry.kind === "mission");
  return groupEntries(kept).map((section) => ({ ...section, entries: section.entries.slice(0, DESK_SEARCH_SHOWN) }));
}
