import { describe, expect, it } from "vitest";
import type { KitAgent, KitPiece, SnapshotRest, Workspace } from "./snapshot";
import { guardPartOf } from "./ProjectGuardrails";
import { DEFAULT_GUARD_PATHS, guardedBase, guardPathRefusal, projectBaseBranch, projectGuardrails, projectIssueSource, projectRepository, projectSettings, readableSettings, settingsTarget, unguardedAgents } from "./projectSettings";

const project = { id: "display-id", device_id: "local", path: "/project", label: "project", branches: ["main", "develop"], default_branch: "main" } as Workspace;
const snapshot = () => ({ ui_state: {
  project_issue_sources: { "/project": "local" }, project_base_branches: { "/project": "develop" },
  project_settings: { original: { version: 1, device_id: "local", root: "/project" },
    remote: { version: 1, device_id: "remote", root: "/project", issue_source: "github" } },
} }) as SnapshotRest;

describe("confirmed scoped project settings", () => {
  it("inherits unset legacy fields and lets explicit automatic choices shadow them", () => {
    const rest = snapshot();
    expect(projectIssueSource(rest, project)).toBe("local");
    expect(projectBaseBranch(rest, project)).toEqual({ branch: "develop", missing: null });
    Object.assign(rest.ui_state!.project_settings!.original!, { issue_source: "auto", base_branch: { mode: "auto" } });
    expect(projectIssueSource(rest, project)).toBe("auto");
    expect(projectBaseBranch(rest, project)).toEqual({ branch: "main", missing: null });
  });

  it("matches device plus main root despite display id or primary checkout changes", () => {
    const rest = snapshot();
    expect(projectSettings(rest, settingsTarget({ ...project, id: "another-visible-id" })).record?.device_id).toBe("local");
    expect(projectIssueSource(rest, { ...project, device_id: "remote" })).toBe("github");
    expect(projectSettings(rest, { ...settingsTarget(project), root: "/project.worktrees/feature" }).record).toBeNull();
  });

  it("reports a removed branch with an existing fallback", () => {
    const rest = snapshot();
    rest.ui_state!.project_settings!.original!.base_branch = { mode: "branch", branch: "deleted" };
    expect(projectBaseBranch(rest, project)).toEqual({ branch: "main", missing: "deleted" });
    expect(projectBaseBranch(rest, { ...project, default_branch: "deleted" })).toEqual({ branch: "main", missing: "deleted" });
  });

  it("finds a failed first-write receipt without a confirmed record", () => {
    const rest = snapshot();
    rest.ui_state!.project_settings = {};
    rest.ui_state!.project_settings_writes = { scope: { device_id: "local", root: "/project", request_id: "intent", state: "failed", error: "project_settings_save_failed", patch: { issue_source: "github" } } };
    expect(projectSettings(rest, settingsTarget(project))).toMatchObject({ record: null, write: { request_id: "intent", patch: { issue_source: "github" } } });
    expect(projectSettings(rest, { ...settingsTarget(project), deviceId: "remote" }).write).toBeNull();
  });
  it("finds a correlated busy refusal while another request is pending", () => {
    const rest = snapshot();
    rest.ui_state!.project_settings_writes = {
      active: { device_id: "local", root: "/project", request_id: "active", state: "pending", error: null },
      refusal: { device_id: "local", root: "/project", request_id: "second", state: "failed", error: "project_settings_busy" },
    };
    expect(projectSettings(rest, settingsTarget(project), "second").write).toMatchObject({ request_id: "second", state: "failed", error: "project_settings_busy" });
    expect(projectSettings(rest, settingsTarget(project), "active").write?.state).toBe("pending");
  });

});

describe("project guardrails", () => {
  it("reads a version 1 record with every protection off and a newer one as unreadable (B30)", () => {
    const record = { version: 1, device_id: "local", root: "/project" };
    expect(projectGuardrails(record)).toEqual({ worktree_folder: false, base_branch: false, agent_merge: false, git_commands: false, files: false, paths: [".env", ".env.*", "*.pem", "*.key", "~/.ssh/"] });
    expect(readableSettings(record)).toBe(true);
    expect(readableSettings({ ...record, version: 2 })).toBe(true);
    expect(readableSettings({ ...record, version: 3 })).toBe(false);
    expect(readableSettings(null)).toBe(true);
  });

  it("refuses a path past the list's limits before it is sent (B16)", () => {
    expect(guardPathRefusal(DEFAULT_GUARD_PATHS, ".aws/")).toBeNull();
    expect(guardPathRefusal(DEFAULT_GUARD_PATHS, ".env")).toBe("project_settings_path_duplicate");
    expect(guardPathRefusal([], "a".repeat(200))).toBeNull();
    expect(guardPathRefusal([], "a".repeat(201))).toBe("project_settings_path_too_long");
    expect(guardPathRefusal(Array.from({ length: 31 }, (_, n) => `p${n}`), "last")).toBeNull();
    expect(guardPathRefusal(Array.from({ length: 32 }, (_, n) => `p${n}`), "one-more")).toBe("project_settings_paths_limit");
    expect(guardPathRefusal([], "  ")).toBe("project_settings_path_invalid");
    // Characters as the core counts them, and the core's shape rule.
    expect(guardPathRefusal([], "경".repeat(200))).toBeNull();
    expect(guardPathRefusal([], "경".repeat(201))).toBe("project_settings_path_too_long");
    for (const path of [" .env", "~", "~/", "~user/x", "# note", "!.env", "a\tb"]) expect(guardPathRefusal([], path), path).toBe("project_settings_path_invalid");
    expect(guardPathRefusal([], "~/.aws/")).toBeNull();
  });

  it("guards the chosen base branch, or the remote's default for Auto, and none when the project names none (B17)", () => {
    const rest = snapshot();
    const record = (base_branch?: { mode: "auto" } | { mode: "branch"; branch: string }) => ({ version: 2, device_id: "local", root: "/project", base_branch });
    expect(guardedBase(rest, project, record({ mode: "branch", branch: "develop" }))).toBe("develop");
    expect(guardedBase(rest, project, record({ mode: "auto" }))).toBe("main");
    expect(guardedBase(rest, project, record())).toBe("develop");
    expect(guardedBase({} as SnapshotRest, { ...project, default_branch: null } as Workspace, record({ mode: "auto" }))).toBeNull();
  });

  it("places a reopened receipt beside the control it moved, and a saved one nowhere", () => {
    const confirmed = projectGuardrails(null);
    expect(guardPartOf(confirmed, confirmed)).toBeUndefined();
    expect(guardPartOf({ ...confirmed, git_commands: true }, confirmed)).toBe("git_commands");
    expect(guardPartOf({ ...confirmed, paths: [...confirmed.paths, ".aws/"] }, confirmed)).toBe("paths");
  });

  it("names the installed agents whose Hide hook is not in place, Codex's untrusted one included (B25)", () => {
    const installed: KitPiece = { state: "installed", why: null, location: null };
    const agent = (id: string, label: string, over: Partial<KitAgent> = {}): KitAgent => ({ id, label, availability: "available", enabled: true, chosen: true, skill: installed, hook: installed, doc_url: "", ...over });
    expect(unguardedAgents([agent("claude-code", "Claude Code"), agent("codex", "Codex")])).toEqual([]);
    const names = unguardedAgents([
      agent("claude-code", "Claude Code"),
      agent("codex", "Codex", { hook: { state: "failed", why: { cause: "codex_untrusted" }, location: null } }),
      agent("grok", "Grok", { enabled: false }),
      agent("omp", "omp", { availability: "not_installed", enabled: false, chosen: false }),
      agent("pi", "Pi", { availability: "unsupported_system" }),
    ]).map((row) => row.id);
    expect(names).toEqual(["codex", "grok"]);
  });

  it("names the GitHub repository a Git project is connected to: the confirmed one, else the detected one, never a folder's", () => {
    const rest = snapshot();
    const git = { ...project, is_git: true } as Workspace;
    expect(projectRepository(rest, git)).toBeNull();
    expect(projectRepository(rest, { ...git, tasks: { source: { kind: "github", name: "acme/tasks" } } } as Workspace)).toBe("acme/tasks");
    expect(projectRepository(rest, { ...git, tasks: { source: { kind: "local", name: "memos" } } } as Workspace)).toBeNull();
    const detected = { ...git, home_issues: { repository: "acme/detected" } } as Workspace;
    expect(projectRepository(rest, detected)).toBe("acme/detected");
    rest.ui_state!.project_settings!.original!.repository = { mode: "repository", repository: "acme/chosen" };
    expect(projectRepository(rest, detected)).toBe("acme/chosen");
    expect(projectRepository(rest, { ...detected, is_git: false } as Workspace)).toBeNull();
  });
});
