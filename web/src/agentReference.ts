import { remoteTargetOfPane } from "./remote";
import { DRAG_CHIP_CLASS, holdShellDrag } from "./shellDrag";
import { localDeviceId, type AgentRow, type SnapshotRest } from "./snapshot";

// An agent reference is the one line that names another pane for the agent
// in this one: `@<name> (pane <id> · <task>)`. The menu copies it, a
// sidebar row dropped on a terminal types it, and a pane header's @ types the
// one chosen from its list into that pane; the agent that reads it gives the
// pane id to `hide pane read`, `hide pane send` and `hide request send`.
// A pane of this machine is named bare (`w9J:p52`); a pane of a connected
// device carries its full address (`remote:<device>:pane:w2:p7`), because a
// bare id names a pane only on the machine of whoever reads it.

/** A task label longer than this is cut, so a reference stays one short line. */
const TASK_LIMIT = 80;

/** One line of text: control characters (a newline would submit the prompt) become spaces. */
function oneLine(text: string, limit: number): string {
  const flat = Array.from(text.replace(/\p{Cc}/gu, " ").replace(/\s+/g, " ").trim());
  // Cut by code point, so an emoji is never split into a lone surrogate.
  return flat.length > limit ? `${flat.slice(0, limit - 1).join("")}…` : flat.join("");
}

/** The reference text for a pane: its name, the id a reader on this machine addresses it by, and the task it does when one is known. */
export function referenceText(name: string, id: string, task?: string | null): string {
  const named = oneLine(name, TASK_LIMIT);
  const doing = task ? oneLine(task, TASK_LIMIT) : "";
  const detail = [`pane ${id}`, doing && doing !== named ? doing : ""].filter(Boolean).join(" · ");
  return named && named !== id ? `@${named} (${detail})` : `@${detail}`;
}

/** An agent's reference: the name the engine knows it by, else its kind, then its task label. */
export function agentReference(agent: Pick<AgentRow, "pane_id" | "herdr_name" | "agent_kind" | "identity_label">): string {
  return referenceText(agent.herdr_name || agent.agent_kind, agent.pane_id, agent.identity_label);
}

/** The device a pane belongs to: its remote target, else this machine. */
export function paneDevice(rest: SnapshotRest | null, paneId: string): string {
  return remoteTargetOfPane(rest, paneId) ?? localDeviceId(rest);
}

/**
 * Whether `reference` can be typed into `target`: another pane on the same
 * device, or a device's pane typed into a pane of this machine, which takes
 * its full address. A bare id of this machine typed into a device's pane
 * would name that device's pane, and a device's address typed into another
 * device's is not one that device can reach.
 */
export function referenceLands(rest: SnapshotRest | null, reference: string, target: string): boolean {
  if (reference === target) return false;
  const to = paneDevice(rest, target);
  return paneDevice(rest, reference) === to || to === localDeviceId(rest);
}

/** One sidebar row's drag: which pane it names and the text a drop types. */
export type AgentDrag = { paneId: string; text: string };
export const agentDrag: { current: AgentDrag | null } = { current: null };

/**
 * The drag ends on the native dragend, which the window hears while the row
 * is still in the page. A row can re-sort away mid-drag, and a removed row
 * fires no dragend, so the first mouse event after the drag (none fire during
 * one) ends it too; a held drag would otherwise keep every hover card shut.
 */
export function beginAgentDrag(agent: Parameters<typeof agentReference>[0], event: { dataTransfer: DataTransfer }): void {
  const item = { paneId: agent.pane_id, text: agentReference(agent) };
  event.dataTransfer.effectAllowed = "copy";
  event.dataTransfer.setData("text/plain", item.text);
  // The row's own button is a transparent cover, so the drag shows the text a drop types.
  const chip = document.createElement("div");
  chip.className = `fixed left-0 top-0 -translate-y-full ${DRAG_CHIP_CLASS}`;
  chip.textContent = item.text;
  document.body.append(chip);
  event.dataTransfer.setDragImage(chip, 0, 0);
  setTimeout(() => chip.remove(), 0);
  agentDrag.current = item;
  // The same mark an Explorer file drag holds, so a browser display gives the drop its place.
  const release = holdShellDrag("file");
  const end = () => {
    window.removeEventListener("dragend", end, true);
    window.removeEventListener("mousemove", end, true);
    window.removeEventListener("pointerdown", end, true);
    if (agentDrag.current === item) agentDrag.current = null;
    release();
  };
  window.addEventListener("dragend", end, true);
  window.addEventListener("mousemove", end, true);
  window.addEventListener("pointerdown", end, true);
}

/**
 * The agents a pane's @ can name, in the order given: every agent whose
 * reference lands in `paneId` (`referenceLands`).
 */
export function mentionCandidates<A extends Pick<AgentRow, "pane_id">>(rest: SnapshotRest | null, agents: readonly A[], paneId: string): A[] {
  return agents.filter((agent) => referenceLands(rest, agent.pane_id, paneId));
}
