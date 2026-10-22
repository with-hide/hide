// The lane's tools, in the rail's order (PRD lane-two-columns D-07, D-11,
// D-14, D-20: the Project tool, then the checkout's Explorer and Changes). Kept apart from `workspace.ts` so the
// shortcut registry, which the desktop host's main process also reads, can
// name them without that module's screen graph.

export const TOOLS = ["project", "explorer", "changes"] as const;
export type Tool = (typeof TOOLS)[number];
