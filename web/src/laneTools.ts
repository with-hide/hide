// The lane's tools, in the rail's order (PRD lane-two-columns D-07, D-11,
// D-14: the Project tool and the Project's Memory, then the checkout's
// Explorer and Changes; Memory is its own tool again by the operator's
// decision of 2026-10-23, replacing D-20). Kept apart from `workspace.ts` so the
// shortcut registry, which the desktop host's main process also reads, can
// name them without that module's screen graph.

export const TOOLS = ["project", "memory", "explorer", "changes"] as const;
export type Tool = (typeof TOOLS)[number];
