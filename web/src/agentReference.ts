import { herdrPaneId, remoteTargetOfPane } from "./remote";
import { DRAG_CHIP_CLASS, holdShellDrag } from "./shellDrag";
import { localDeviceId, type AgentRow, type SnapshotRest } from "./snapshot";

// An agent reference is the one line that names another pane for the agent
// in this one: `@<name> (pane <id> · <task>)`. The menu copies it, a
// sidebar row dropped on a terminal types it, and a pane header's @ types the
// one chosen from its list into that pane; the agent that reads it sends
// its letter to the pane id, which `hide request send` takes on the machine
// that owns the pane.

/** A task label longer than this is cut, so a reference stays one short line. */
const TASK_LIMIT = 80;

/** One line of text: control characters (a newline would submit the prompt) become spaces. */
function oneLine(text: string, limit: number): string {
  const flat = Array.from(text.replace(/\p{Cc}/gu, " ").replace(/\s+/g, " ").trim());
  // Cut by code point, so an emoji is never split into a lone surrogate.
  return flat.length > limit ? `${flat.slice(0, limit - 1).join("")}…` : flat.join("");
}

/** The reference text for a pane: its name, its Herdr pane id, and the task it does when one is known. */
export function referenceText(name: string, paneId: string, task?: string | null): string {
  const id = herdrPaneId(paneId);
  const named = oneLine(name, TASK_LIMIT);
  const doing = task ? oneLine(task, TASK_LIMIT) : "";
  const detail = [`pane ${id}`, doing && doing !== named ? doing : ""].filter(Boolean).join(" · ");
  return named && named !== id ? `@${named} (${detail})` : `@${detail}`;
}

/** An agent's reference: the name Herdr knows it by, else its kind, then its task label. */
export function agentReference(agent: Pick<AgentRow, "pane_id" | "herdr_name" | "agent_kind" | "identity_label">): string {
  return referenceText(agent.herdr_name || agent.agent_kind, agent.pane_id, agent.identity_label);
}

/** The device a pane belongs to: its remote target, else this machine. */
export function paneDevice(rest: SnapshotRest | null, paneId: string): string {
  return remoteTargetOfPane(rest, paneId) ?? localDeviceId(rest);
}

/**
 * Whether `reference` can be typed into `target`: another pane on the same
 * device, because a pane id names a pane only on the machine that owns it.
 */
export function referenceLands(rest: SnapshotRest | null, reference: string, target: string): boolean {
  return reference !== target && paneDevice(rest, reference) === paneDevice(rest, target);
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
 * reference lands in `paneId` (another pane on the same device).
 */
export function mentionCandidates<A extends Pick<AgentRow, "pane_id">>(rest: SnapshotRest | null, agents: readonly A[], paneId: string): A[] {
  return agents.filter((agent) => referenceLands(rest, agent.pane_id, paneId));
}
