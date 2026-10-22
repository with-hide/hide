// The lane's tools, in the rail's order (PRD lane-two-columns D-07, D-11;
// PRD sidebar-task-lens D-16, D-17: the Project's Memory and PRs, then the
// checkout's Explorer and Changes). Kept apart from `workspace.ts` so the
// shortcut registry, which the desktop host's main process also reads, can
// name them without that module's screen graph.

export const TOOLS = ["memory", "prs", "explorer", "changes"] as const;
export type Tool = (typeof TOOLS)[number];
