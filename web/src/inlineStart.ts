import type { AgentStartReceipt, AgentStartTarget } from "./agentLayout";
import { findArea, locateDisplay } from "./areaLayout";
import { catalogWorkspaces, type SnapshotRest } from "./snapshot";
import { startAnswer, type StartAnswer } from "./startAnswer";

export const receiptTarget = (receipt: AgentStartReceipt): AgentStartTarget => ({ group_id: receipt.group_id, area_id: receipt.area_id, tab_id: receipt.draft_tab_id ?? (receipt.add_pane ? receipt.live_tab_id : null), add_pane: receipt.add_pane });
export function inlineReceipts(rest: SnapshotRest | null) {
  return catalogWorkspaces(rest).flatMap((workspace) => workspace.checkouts.flatMap((checkout) => (checkout.agent_groups?.start_receipts ?? []).map((receipt) => ({ device: workspace.device_id, path: checkout.path, receipt }))));
}

/** Only a definite failure can be abandoned after the core moves its target. */
export function failedReceiptMoved(rest: SnapshotRest | null, device: string, path: string, receipt: AgentStartReceipt): boolean {
  if (receipt.state !== "failed") return false;
  const checkout = catalogWorkspaces(rest).find((workspace) => workspace.device_id === device && workspace.checkouts.some((checkout) => checkout.path === path))?.checkouts.find((checkout) => checkout.path === path);
  const groups = checkout?.agent_groups;
  if (!groups) return false;
  const target = receiptTarget(receipt);
  const group = groups.groups.find((row) => row.id === target.group_id);
  // Delegated canvases intentionally have no strip membership. The core keeps
  // their admitted add intent valid while its original group and area survive.
  const delegated = target.add_pane && checkout!.tabs.some((tab) => tab.id === target.tab_id && tab.delegated);
  if (!group || (target.tab_id !== null && !group.tabs.includes(target.tab_id) && !delegated)) return true;
  const front = rest?.workspace_view;
  const layout = front?.device_id === device && front.path === path ? front.agent_layout : null;
  if (!layout || layout.group_id !== target.group_id) return false;
  if (target.tab_id === null || delegated) return !findArea(layout.root, target.area_id);
  const located = locateDisplay(layout.root, target.tab_id);
  return located !== null && located.area.id !== target.area_id;
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
