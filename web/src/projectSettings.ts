import type { KitAgent, SnapshotRest, Workspace } from "./snapshot";
import { agentInstalled, agentUnsupportedHere, supportedAgents } from "./settings/agentRows";

export type RepositoryConnection = { mode: "auto" } | { mode: "repository"; repository: string };
export type BaseBranchChoice = { mode: "auto" } | { mode: "branch"; branch: string };
/** The protections of Working style (`hide_agent_hooks::guardrails::Rules`); every one starts off. */
export type Guardrails = {
  worktree_folder: boolean; base_branch: boolean; agent_merge: boolean; git_commands: boolean; files: boolean; paths: string[];
};
export type ProjectSettings = {
  version: number; device_id: string; root: string;
  repository?: RepositoryConnection; issue_source?: "auto" | "github" | "local";
  new_work?: "checkout" | "worktree"; base_branch?: BaseBranchChoice; auto_fetch?: boolean; guardrails?: Guardrails;
};
export type ProjectSettingsPatch = Partial<Pick<ProjectSettings, "repository" | "issue_source" | "new_work" | "base_branch" | "auto_fetch" | "guardrails">>;

/** The newest record this build reads and writes; version 1 is the same record without guardrails (PRD agent-guardrails D-23). */
export const PROJECT_SETTINGS_VERSION = 2;
export const readableSettings = (record: ProjectSettings | null) => record === null || (record.version >= 1 && record.version <= PROJECT_SETTINGS_VERSION);

/** The paths a new list starts with (B15), and the limits the core refuses past (B16). */
export const DEFAULT_GUARD_PATHS = [".env", ".env.*", "*.pem", "*.key", "~/.ssh/"];
export const GUARD_PATH_LIMIT = 32;
export const GUARD_PATH_LENGTH = 200;

export function projectGuardrails(record: ProjectSettings | null): Guardrails {
  return record?.guardrails ?? { worktree_folder: false, base_branch: false, agent_merge: false, git_commands: false, files: false, paths: [...DEFAULT_GUARD_PATHS] };
}

/** Why `path` cannot join `paths`, as the core would answer, or null; the pattern itself is the core's to judge. */
export function guardPathRefusal(paths: readonly string[], path: string): "project_settings_paths_limit" | "project_settings_path_too_long" | "project_settings_path_duplicate" | "project_settings_path_invalid" | null {
  if (!guardPathShaped(path)) return "project_settings_path_invalid";
  if (paths.length >= GUARD_PATH_LIMIT) return "project_settings_paths_limit";
  // Characters, as the core counts them (`chars`), not UTF-16 units.
  if ([...path].length > GUARD_PATH_LENGTH) return "project_settings_path_too_long";
  if (paths.includes(path)) return "project_settings_path_duplicate";
  return null;
}

/** The core's shape rule for one entry (`Rules::validate`): trimmed, no control character, a `~/` path under home
 * or a pattern, never only a comment or a negation. A pattern the gitignore reader refuses is the core's to say. */
function guardPathShaped(path: string): boolean {
  if (!path.trim() || path !== path.trim() || /\p{Cc}/u.test(path)) return false;
  if (path.startsWith("~") && (!path.startsWith("~/") || path === "~/")) return false;
  return !path.startsWith("#") && !path.startsWith("!");
}

/** A save the core refused for what it asked, which sending it again cannot change. */
export function refusedForGood(code: string | null | undefined): boolean {
  return code === "project_settings_paths_limit" || code === "project_settings_path_too_long" || code === "project_settings_path_duplicate"
    || code === "project_settings_path_invalid" || code === "project_settings_not_git" || code === "project_settings_base_unknown"
    || code === "project_settings_invalid_repository" || code === "project_settings_newer_version";
}

/** The branch the base protection guards: the chosen one, or the remote's default for Auto; null when the project names none (B17). */
export function guardedBase(rest: SnapshotRest | null, workspace: Workspace, record: ProjectSettings | null): string | null {
  const choice = record?.base_branch;
  if (choice?.mode === "branch") return choice.branch;
  const legacy = choice?.mode === "auto" ? undefined : rest?.ui_state?.project_base_branches?.[workspace.path];
  return legacy ?? workspace.default_branch ?? null;
}

/**
 * The agents of the project's machine the guardrails do not reach (B25): installed there, and switched off in Hide
 * or with a hook that is not in place, Codex's untrusted hook included. An agent the system cannot run is no gap.
 */
export function unguardedAgents(agents: readonly KitAgent[] | undefined): KitAgent[] {
  return supportedAgents(agents).filter((agent) => agentInstalled(agent) && !agentUnsupportedHere(agent) && !(agent.enabled && agent.hook?.state === "installed"));
}
export type ProjectSettingsWrite = { request_id: string; state: "pending" | "saved" | "failed"; error: string | null; patch?: ProjectSettingsPatch };
export type ProjectSettingsTarget = { deviceId: string; root: string; workspaceId: string; label: string };

export const settingsTarget = (workspace: Workspace): ProjectSettingsTarget => ({ deviceId: workspace.device_id, root: workspace.path, workspaceId: workspace.id, label: workspace.label });

/** The durable identity belongs to the core. Match its tuple, never a visible checkout id. */
export function projectSettings(rest: SnapshotRest | null, target: ProjectSettingsTarget, requestId?: string): { record: ProjectSettings | null; write: ProjectSettingsWrite | null } {
  const entry = Object.entries(rest?.ui_state?.project_settings ?? {}).find(([, row]) => row.device_id === target.deviceId && row.root === target.root);
  const record = entry?.[1] ?? null;
  // A failed first write has no saved record yet; receipts carry their scope too.
  const write = Object.values(rest?.ui_state?.project_settings_writes ?? {}).find((row) => row.device_id === target.deviceId && row.root === target.root && (!requestId || row.request_id === requestId)) ?? null;
  return { record, write };
}

/** The repository the core detected for a Git project: its issue source when that is GitHub, else the one its remote names. */
export const detectedRepository = (workspace: Workspace): string | null =>
  workspace.home_issues?.repository ?? (workspace.tasks?.source?.kind === "github" ? workspace.tasks.source.name : null);

/**
 * The GitHub repository a project is connected to as `owner/repo`: the one the operator confirmed in Project Settings,
 * else the one detected. A folder has none.
 */
export function projectRepository(rest: SnapshotRest | null, workspace: Workspace): string | null {
  if (!workspace.is_git) return null;
  const confirmed = projectSettings(rest, settingsTarget(workspace)).record?.repository;
  return confirmed?.mode === "repository" ? confirmed.repository : detectedRepository(workspace);
}

export function projectIssueSource(rest: SnapshotRest | null, workspace: Workspace): string | undefined {
  return projectSettings(rest, settingsTarget(workspace)).record?.issue_source ?? rest?.ui_state?.project_issue_sources?.[workspace.path];
}

/** New worktrees use the same confirmed override as Git comparison; a deleted branch falls back visibly. */
export function projectBaseBranch(rest: SnapshotRest | null, workspace: Workspace): { branch: string; missing: string | null } {
  const record = projectSettings(rest, settingsTarget(workspace)).record;
  const choice = record?.base_branch;
  const legacy = rest?.ui_state?.project_base_branches?.[workspace.path];
  const wanted = choice?.mode === "branch" ? choice.branch : choice?.mode === "auto" ? null : legacy;
  const branches = workspace.branches ?? [];
  const fallback = workspace.default_branch && branches.includes(workspace.default_branch) ? workspace.default_branch : branches[0] ?? "";
  return { branch: wanted && branches.includes(wanted) ? wanted : fallback, missing: wanted && !branches.includes(wanted) ? wanted : null };
}
