// What Add a project's importable list shows (PRD device-add-project D-09 to
// D-13): the projects a machine's node found worth adding, less those
// registered there or added from the list since, and the name each source
// chip carries.

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
