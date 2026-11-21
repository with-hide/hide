// What Add a project's importable list shows (PRD device-add-project D-09 to
// D-13): the projects a machine's node found worth adding, less those
// registered there or added from the list since, the name each source goes
// by, and the source chips that filter the rows.

import { agentProductName } from "./agentAdapters";
import { alreadyRegistered } from "./addProject";
import type { WorkspaceRegistration } from "./snapshot";
import type { ImportableAnswer, ImportableProject, ImportSource } from "./store";

/** The apps whose own lists a node reads, by the name they go by; every other source is an agent. */
const APP_NAMES: Readonly<Record<string, string>> = { paseo: "Paseo", herdr: "Herdr", cursor: "Cursor" };

/** The name a source chip shows: the app's, or the agent's product name ("Claude Code", "Codex"). */
export function sourceName(source: ImportSource): string {
  return APP_NAMES[source] ?? agentProductName(source);
}

/** Whether a source is an app's list rather than an agent's sessions; the two chips look apart. */
export function sourceIsApp(source: ImportSource): boolean {
  return source === "paseo" || source === "herdr";
}

/**
 * The rows to show for `device` from an answer newer than the ask (`after`): none while the node is
 * still looking or could not be asked, and none of the projects registered there, so a row added from
 * the list leaves it once its registration lands.
 */
export function importRows(
  answer: ImportableAnswer | undefined,
  after: number,
  device: string,
  registrations: readonly WorkspaceRegistration[],
): ImportableProject[] {
  if (!answer || answer.seq <= after || !answer.projects) return [];
  return answer.projects.filter((project) => !alreadyRegistered(project.path, device, registrations));
}

/** One source chip above the rows: the source and how many rows it named. */
export type ImportFilter = { source: ImportSource; count: number };

/**
 * The source chips for `rows`, the most rows first and the order the node named them in a tie; none
 * when every row came from one source, since a chip would then hide nothing.
 */
export function importFilters(rows: readonly ImportableProject[]): ImportFilter[] {
  const counts = new Map<ImportSource, number>();
  for (const row of rows) for (const source of row.sources) counts.set(source, (counts.get(source) ?? 0) + 1);
  if (counts.size < 2) return [];
  return [...counts].map(([source, count]) => ({ source, count })).sort((a, b) => b.count - a.count);
}

/** The rows a chip shows: every row for `null`, else the rows that source named. */
export function filterRows(rows: readonly ImportableProject[], source: ImportSource | null): ImportableProject[] {
  return source === null ? [...rows] : rows.filter((row) => row.sources.includes(source));
}

/** The folder a row's project sits in, `~/projects` for `~/projects/app`: the row's name already says the last part. */
export function parentLabel(label: string): string {
  const cut = label.lastIndexOf("/");
  return cut <= 0 ? label : label.slice(0, cut);
}
