import type { AgentStartReceipt, AgentStartTarget } from "./agentLayout";
import { catalogWorkspaces, type SnapshotRest } from "./snapshot";
import { startAnswer, type StartAnswer } from "./startAnswer";

export const receiptTarget = (receipt: AgentStartReceipt): AgentStartTarget => ({ group_id: receipt.group_id, area_id: receipt.area_id, tab_id: receipt.draft_tab_id ?? (receipt.add_pane ? receipt.live_tab_id : null), add_pane: receipt.add_pane });
export function inlineReceipts(rest: SnapshotRest | null) {
  return catalogWorkspaces(rest).flatMap((workspace) => workspace.checkouts.flatMap((checkout) => (checkout.agent_groups?.start_receipts ?? []).map((receipt) => ({ device: workspace.device_id, path: checkout.path, receipt }))));
}

/** A durable topology receipt takes precedence over an old transient refusal. */
export function inlineStartAnswer(rest: SnapshotRest | null, requestId: string, receipt: AgentStartReceipt | undefined): StartAnswer {
  if (!receipt) return startAnswer(rest, requestId);
  const operation = rest?.task_operation;
  const task = operation?.request_id === requestId ? operation : null;
  if (receipt.state === "pending") return { phase: "pending" };
  if (receipt.state === "failed") return { phase: "failed", message: task?.message ?? null };
  if (task?.phase === "ready") return { phase: "ready", taskId: task.id, paneId: receipt.pane_id, agentPhase: task.agent_phase, agentMessage: task.agent_message };
  // No task slot remains after restart. The same intent reconstructs it before retry.
  if (receipt.agent_phase && receipt.agent_phase !== "started") return { phase: "pending" };
  return { phase: "ready", taskId: 0, paneId: receipt.pane_id, agentPhase: receipt.agent_phase, agentMessage: null };
}
