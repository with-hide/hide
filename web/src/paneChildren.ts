import { childrenOf } from "./components/agent-tree-popover";
import { agentsByPane } from "./paneAgents";
import { deviceConnected, localDeviceId } from "./devices";
import type { MessageKey } from "./i18n/catalogs";
import { remoteTargetOfPane } from "./remote";
import type { AgentRow, RowMark, SnapshotRest } from "./snapshot";

export type ChildCountPart = {
  kind: "raised" | "working" | "result" | "waiting" | "unknown" | "idle";
  /** The mark the count is drawn with: a resting count (idle, a child's own ask) draws the quiet ring its rows draw, an unknown one none; the hint names each. */
  mark: RowMark | null;
  count: number;
  label: Extract<MessageKey, `panes.children.${string}`>;
};

/** Presentation of the core's direct-child counts, not another status projection. */
export function paneChildren(paneId: string, agents: AgentRow[], rest: SnapshotRest | null) {
  const byPane = agentsByPane(agents);
  const parent = byPane.get(paneId);
  if (!parent?.lineage_child_pane_ids?.length) return null;
  const children = childrenOf(parent, byPane);
  if (children.length === 0) return null;
  // Topology can survive a connection. Subtree totals cannot claim current
  // work when one of their owning devices is unavailable.
  const unavailable = agents.find((agent) => {
    if (agent.pane_id !== paneId && agent.lineage_parent_pane_id !== paneId && !agent.lineage_path_pane_ids?.includes(paneId)) return false;
    return !deviceConnected(rest, agent.device_id ?? remoteTargetOfPane(rest, agent.pane_id) ?? localDeviceId(rest));
  });
  const unavailableDevice = unavailable ? unavailable.device_id ?? remoteTargetOfPane(rest, unavailable.pane_id) ?? localDeviceId(rest) : null;
  const raised = parent.descendant_mark?.kind === "raised" ? parent.descendant_mark.count : 0;
  const parts: ChildCountPart[] = [];
  const counts = parent.direct_child_counts;
  // The four marks mean here what they mean on a row (B21, B24): a request a
  // child raised to the operator is `!`; a child's own question is its
  // parent's to answer, so it draws the quiet ring its row draws.
  const asking = counts ? counts.error + counts.approval + counts.question : 0;
  if (unavailableDevice === null) {
    parts.push({ kind: "raised", mark: "turn", count: raised, label: "panes.children.raised" }, { kind: "waiting", mark: "quiet", count: asking, label: "panes.children.waiting" });
  }
  if (counts && unavailableDevice === null) {
    // The core omits its unknown count from the wire; its row mark already
    // applies demand precedence and names these children (D-04).
    const unknown = children.filter((child) => child.mark === "unknown").length;
    const idle = children.length - asking - counts.working - counts.done - unknown;
    parts.push(
      { kind: "working", mark: "going", count: counts.working, label: "panes.children.working" },
      { kind: "result", mark: "unread", count: counts.done, label: "panes.children.result" },
      { kind: "unknown", mark: "unknown", count: unknown, label: "panes.children.unknown" },
      { kind: "idle", mark: "quiet", count: idle, label: "panes.children.idle" },
    );
  }
  return { parent, children, raised, parts: parts.filter((part) => part.count > 0), lead: children[0]!, unavailableDevice };
}

export type PaneChildren = NonNullable<ReturnType<typeof paneChildren>>;
