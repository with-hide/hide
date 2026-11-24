// ⌘P and ⌘K while Desk is in front (PRD desk-2-ui D-23, D-24, B37-B39): where
// ⌘P searches, how an all-projects answer is put in order, and what a ⌘K
// pick opens in the tab panel. The palettes themselves are the app's own;
// these are the decisions Desk adds, kept as functions of values so they are
// testable without a browser.

import type { DeskItem } from "./desk";
import type { DeskTab } from "./deskRoom";
import { checkoutAtRoot, checkoutOfPaneIn, listedCheckouts } from "./navigation";
import { groupEntries, type SearchEntry, type SearchSection } from "./search";
import type { SnapshotRest } from "./snapshot";

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
  const found = checkoutOfPaneIn(rest, session);
  return found ? { kind: "checkout", device: found.device, root: found.checkout.path } : { kind: "all" };
}

/**
 * The project Desk named last (D-23): the newest conversation line that
 * names a project, or a session whose project it is. A project is named in
 * the `@` menu by its primary checkout's path, a session by its pane.
 */
export function lastNamedWorkspace(rest: SnapshotRest | null, items: readonly DeskItem[]): string | null {
  const rows = listedCheckouts(rest);
  const ofProject = (id: string) => rows.find((row) => row.workspace.id === id || row.workspace.path === id || row.workspace.checkouts.some((checkout) => checkout.path === id))?.workspace.id ?? null;
  const ofPane = (pane: string) => checkoutOfPaneIn(rest, pane)?.workspace.id ?? null;
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
  const found = listedCheckouts(rest).find((row) => row.workspace.id === workspaceId);
  return found ? { device_id: found.device, root_path: found.workspace.path } : null;
}

/** One all-projects result as its row draws it: the muted `project ›`, the file's name and its folder. */
export type ProjectFileRow = { hit: DeskFileHit; project: string; name: string; folder: string };

/**
 * The answer in the order the list shows (B38): the named project's files
 * first, then the others, each keeping the rank hided gave it. A file of a
 * checkout that is no longer listed is left out, since nothing could open it.
 */
export function projectFileRows(rest: SnapshotRest | null, files: readonly DeskFileHit[], named: string | null): { named: ProjectFileRow[]; other: ProjectFileRow[] } {
  const first: ProjectFileRow[] = [];
  const other: ProjectFileRow[] = [];
  for (const hit of files) {
    const place = checkoutAtRoot(rest, hit.device_id, hit.root_path);
    if (!place) continue;
    const parts = hit.relative_path.split("/");
    const row = { hit, project: place.workspace.label, name: parts.at(-1) ?? hit.relative_path, folder: parts.slice(0, -1).join("/") };
    (place.workspace.id === named ? first : other).push(row);
  }
  return { named: first, other };
}

/** The tab ⌘K opens for a PR or a mission row (B39), null for any other kind of row. */
export function deskTabOf(entry: SearchEntry): DeskTab | null {
  if (entry.kind === "mission" && entry.missionId) return { kind: "mission", mission: entry.missionId, title: entry.title };
  if (entry.kind === "pr" && !entry.external && entry.pr && entry.workspaceId) {
    return {
      kind: "output",
      output: { id: `pr:${entry.workspaceId}:${entry.pr.number}`, kind: "pr", label: `#${entry.pr.number}`, thumb: null, seen: true, pr_state: entry.pr.state, device_id: null, checkout: null, path: null, url: entry.pr.url },
    };
  }
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
