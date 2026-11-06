// The desktop's answer to `escalationOf` (`turnItems`, `missionEntry`,
// `summaryGroups`): a node's worker row, found by its pane among this
// machine's and each device's agents, read for the escalation the core
// raised. The phone reads the same escalation from the node hided sent.

import { useMemo } from "react";
import { workerEscalation, type WorkerEscalation } from "./missions";
import { agentsByPane, allAgents } from "./paneAgents";
import type { MissionNode } from "./snapshot";
import { useShellStore } from "./store";

/** The escalation of each node's worker; a new function only when an agent row moved. */
export function useWorkerEscalation(): (node: Pick<MissionNode, "pane_id">) => WorkerEscalation | null {
  const local = useShellStore((s) => s.agents);
  const remote = useShellStore((s) => s.rest?.status?.remote);
  return useMemo(() => {
    const rows = agentsByPane(allAgents(local, remote));
    return (node) => workerEscalation(node.pane_id ? rows.get(node.pane_id) : null);
  }, [local, remote]);
}
