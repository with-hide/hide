import type { AgentStartReceipt, AgentStartTarget } from "./agentLayout";
import { findArea, locateDisplay } from "./areaLayout";
import { catalogWorkspaces, type SnapshotRest, type TaskOperation } from "./snapshot";
import { startAnswer, type StartAnswer } from "./startAnswer";

export const receiptTarget = (receipt: AgentStartReceipt): AgentStartTarget => ({ group_id: receipt.group_id, area_id: receipt.area_id, tab_id: receipt.draft_tab_id ?? (receipt.add_pane ? receipt.live_tab_id : null), add_pane: receipt.add_pane });
export function inlineReceipts(rest: SnapshotRest | null) {
  return catalogWorkspaces(rest).flatMap((workspace) => workspace.checkouts.flatMap((checkout) => (checkout.agent_groups?.start_receipts ?? []).map((receipt) => ({ device: workspace.device_id, path: checkout.path, receipt }))));
}

/**
 * A live tab's start whose agent the core could not confirm or saw fail, and
 * that the operator has not dismissed: the start notice's to show, since the
 * tab itself shows its terminal. The intent's latest answer decides what it
 * says: `answering`, the task now answering it with an unconfirmed or failed
 * agent, takes over as the task notice; otherwise the saved receipt is drawn,
 * with `failedBy` and the core's `failureCode` when its last check failed or
 * was refused, and `checking` while a check runs. Only values the snapshot
 * holds are returned, so a store selector can compare them. A check whose agent started ends the notice.
 */
export function liveStartNotice(rest: SnapshotRest | null, dismissed: ReadonlySet<string>): { device: string; path: string; receipt: AgentStartReceipt; answering: TaskOperation | null; failedBy: "check" | "refusal" | null; failureCode: string | null; checking: boolean } | null {
  const operation = rest?.task_operation ?? null;
  const error = rest?.status?.last_error ?? null;
  for (const { device, path, receipt } of inlineReceipts(rest)) {
    if (receipt.live_tab_id === null || dismissed.has(receipt.request_id)) continue;
    if (receipt.state !== "ready" || (receipt.agent_phase !== "unknown" && receipt.agent_phase !== "failed")) continue;
    const notice = { device, path, receipt, answering: null, failedBy: null, failureCode: null, checking: false };
    // The task slot holds the intent's newest answer; a refusal reaches only the error slot.
    if (operation?.request_id === receipt.request_id) {
      if (operation.agent_phase === "started") continue;
      if (operation.agent_phase === "unknown" || operation.agent_phase === "failed") return { ...notice, answering: operation };
      // A failed check stays in the slot; a later check's refusal reaches only the error slot and is the newer answer, as in `startAnswer`.
      if (operation.phase === "failed") return error?.request_id === receipt.request_id ? { ...notice, failedBy: "refusal" as const, failureCode: error.kind } : { ...notice, failedBy: "check" as const, failureCode: operation.failure };
      return { ...notice, checking: true };
    }
    if (error?.request_id === receipt.request_id) return { ...notice, failedBy: "refusal" as const, failureCode: error.kind };
    return notice;
  }
  return null;
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
export function inlineStartAnswer(rest: SnapshotRest | null, requestId: string, receipt: AgentStartReceipt | undefined): StartAnswer | { phase: "unknown"; taskId: number; message: string | null } {
  if (!receipt) return startAnswer(rest, requestId);
  const operation = rest?.task_operation;
  const task = operation?.request_id === requestId ? operation : null;
  if (receipt.state === "pending") {
    // A failed confirmation does not prove the original effect failed. Surface
    // its answer now while keeping the same intent limited to status checks.
    if (task?.kind === "agent_start" && task.phase === "failed") return { phase: "unknown", taskId: task.id, message: task.failure };
    return { phase: "pending" };
  }
  if (receipt.state === "failed") return { phase: "failed", message: task?.failure ?? null };
  if (task?.phase === "ready") return { phase: "ready", taskId: task.id, paneId: receipt.pane_id, deviceId: task.device_id ?? null, agentPhase: task.agent_phase, agentReason: task.agent_reason, agentLine: task.agent_line };
  // No task slot remains after restart. The same intent reconstructs it before retry.
  if (receipt.agent_phase && receipt.agent_phase !== "started") return { phase: "pending" };
  return { phase: "ready", taskId: 0, paneId: receipt.pane_id, deviceId: null, agentPhase: receipt.agent_phase, agentReason: null, agentLine: null };
}
