// A MacBook project after the core moved to the Mac mini (PRD
// core-host-node-move D-29), on invented data: the MacBook dials the core
// (`dials_in`), and its projects reach the window in the MacBook's session as
// the core's node projection draws them, with an issue source, a size the core
// measured on the node and a merged worktree to clean up. The `node-project`
// scene draws it. Every name is example data.

import type { DiskCleanup, ProjectTasks, SnapshotRest, Task, Workspace } from "../snapshot";
import { coreLinkRest, NODE_DEVICE } from "./coreLinkSceneData";
import type { SceneContent } from "./sceneData";

/** The MacBook's project the scene and the test open. */
export const NODE_PROJECT = "remote:mini:workspace:web";

export type NodeIssueSource = "github" | "local";

const HOUR = 3_600_000;

function tasks(source: NodeIssueSource, nowMs: number): ProjectTasks {
  const at = (hours: number) => nowMs - hours * HOUR;
  if (source === "github") {
    const issue = (number: number, title: string, hours: number, labels: Task["labels"] = []): Task => ({ key: `github:acme/web#${number}`, source: "github", id: `#${number}`, url: `https://github.com/acme/web/issues/${number}`, title, open: true, updated_at_unix_ms: at(hours), created_at_unix_ms: at(hours + 24), labels });
    return {
      source: { kind: "github", label: "GitHub", name: "acme/web", reading: false, failure: null, last_read_at_unix_ms: at(0.1) },
      tasks: [issue(12, "Sidebar keeps the old project after a move", 30, [{ name: "bug", color: "d73a4a" }]), issue(9, "Show the machine each project runs on", 80)],
      overflow: false,
    };
  }
  const issue = (number: number, title: string, hours: number): Task => ({ key: `local:/srv/web#${number}`, source: "local", id: `L-${number}`, url: null, title, open: true, updated_at_unix_ms: at(hours), created_at_unix_ms: at(hours) });
  return {
    source: { kind: "local", label: "Local", name: null, reading: false, failure: null, last_read_at_unix_ms: null },
    tasks: [issue(2, "Keep Local issues when the core moves", 2), issue(1, "Write the move-back checklist", 26)],
    overflow: false,
  };
}

/** The review the core keeps for the MacBook's project once the sheet asked for one, checked on the node. */
function nodeCleanup(project: Workspace): DiskCleanup {
  const row = (path: string, branch: string, main: boolean) => ({ path, branch, head: "4f2c9e1", is_main: main, exclusion_code: main ? ("main" as const) : null, exclusion_count: null, in_use: null, pane_count: 0, result: null, result_code: null, bytes: null });
  return {
    id: 1,
    workspace_id: project.id,
    repository_root: project.path,
    phase: "review",
    main_head: "4f2c9e1",
    message: null,
    usage_error: null,
    usage_ready: true,
    free_bytes: 180_000_000_000,
    progress: null,
    rows: [row(project.path, "main", true), row(`${project.path}.worktrees/fix-sidebar`, "fix-sidebar", false)],
    cell_results: [],
    free_before: null,
    free_after: null,
  };
}

/** The MacBook's `web` project as the core projects a node's: its own issue source, a size and a merged worktree. */
function nodeProject(project: Workspace, source: NodeIssueSource, reviewed: boolean, nowMs: number): Workspace {
  const main = project.checkouts[0]!;
  const merged = {
    ...main,
    id: `${project.id}:fix-sidebar`,
    label: "fix-sidebar",
    path: `${project.path}.worktrees/fix-sidebar`,
    branch: "fix-sidebar",
    is_worktree: true,
    is_primary: false,
    has_panes: false,
    landed: true,
    tabs: [],
    strip: [],
    agent_summary: undefined,
    worktree: main.worktree ? { ...main.worktree, path: `${project.path}.worktrees/fix-sidebar`, branch: "fix-sidebar", is_main: false } : main.worktree,
  };
  return {
    ...project,
    remote_target_id: NODE_DEVICE,
    is_git: true,
    tasks: tasks(source, nowMs),
    checkouts: [main, merged],
    cleanup: reviewed ? nodeCleanup(project) : null,
    disk: { total_bytes: 2_400_000_000, unavailable_reason: null, measuring: false, free_bytes: 180_000_000_000, confirmed_bytes: 2_400_000_000, layers: { build_cache: 1_300_000_000, dependencies: 700_000_000, source: 300_000_000, other: 60_000_000, shared_git: 40_000_000 } },
  };
}

/**
 * The window's snapshot with the MacBook's `web` project reading `source`,
 * and its cleanup review once `reviewed`: the core-link scene's node side,
 * whose `mini` device stands for the MacBook dialing in.
 */
export function nodeProjectRest(source: NodeIssueSource, reviewed: boolean, content: SceneContent, nowMs: number): { rest: SnapshotRest; agents: ReturnType<typeof coreLinkRest>["agents"] } {
  const scene = coreLinkRest("node", content, nowMs);
  const remote = (scene.rest.status?.remote ?? []).map((row) =>
    row.target_id === NODE_DEVICE && row.session
      ? { ...row, session: { ...row.session, workspaces: row.session.workspaces.map((project) => (project.id === NODE_PROJECT ? nodeProject(project, source, reviewed, nowMs) : project)) } }
      : row,
  );
  return { rest: { ...scene.rest, status: { ...scene.rest.status, remote } } as SnapshotRest, agents: scene.agents };
}
