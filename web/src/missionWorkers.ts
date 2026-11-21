// The desktop's answer to `escalationOf` (`turnItems`, `missionEntry`,
// `summaryGroups`): a node's worker row, found by its pane among this
// machine's and each device's agents, read for the escalation the core
// raised. The phone reads the same escalation from the node hided sent.

import { useMemo } from "react";
import { workerEscalation, type WorkerEscalation } from "./missions";
import { agentsByPane, allAgents } from "./paneAgents";
import type { HumanTurnLead, MissionNode } from "./snapshot";
import { useShellStore } from "./store";

/** Every agent row by its pane, this machine's and each device's; a new map only when an agent row moved. */
function useWorkerRows() {
  const local = useShellStore((s) => s.agents);
  const remote = useShellStore((s) => s.rest?.status?.remote);
  return useMemo(() => agentsByPane(allAgents(local, remote)), [local, remote]);
}

/** The escalation of each node's worker; a new function only when an agent row moved. */
export function useWorkerEscalation(): (node: Pick<MissionNode, "pane_id">) => WorkerEscalation | null {
  const rows = useWorkerRows();
  return useMemo(() => (node) => workerEscalation(node.pane_id ? rows.get(node.pane_id) : null), [rows]);
}

/** The person's turn a node's worker row leads with, the declaration Done on the map's 내 차례 settles. */
export function useWorkerHumanTurn(node: Pick<MissionNode, "pane_id">): HumanTurnLead | null {
  const rows = useWorkerRows();
  return node.pane_id ? (rows.get(node.pane_id)?.human_turn ?? null) : null;
}
