import type { Mission, MissionNode } from "../snapshot";

/**
 * The gallery's missions with each planned node's `waiting_on` as the core
 * answers it (`runtime::missions::waiting_on`): the decision that names it,
 * else a dropped node before it, else the nodes before it not done, else
 * its start. A node that already says what it waits on keeps it.
 */
export function withCoreWaits(mission: Mission): Mission {
  const nodes = mission.nodes.map((node): MissionNode => {
    if (node.status !== "planned" || node.attention || node.waiting_on !== undefined) return node;
    if (mission.decision && mission.question?.node === node.key) return { ...node, waiting_on: { kind: "decision" } };
    if (node.blocked) return { ...node, waiting_on: { kind: "blocked" } };
    const keys = node.depends_on.filter((key) => mission.nodes.some((other) => other.key === key && other.status !== "done"));
    return { ...node, waiting_on: keys.length > 0 ? { kind: "nodes", keys } : { kind: "start" } };
  });
  return { ...mission, nodes };
}
