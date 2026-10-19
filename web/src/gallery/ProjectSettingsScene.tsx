// Real settings components over invented catalog/provider answers. This scene
// proves rendered states; daemon persistence is exercised in an isolated run.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { InterfaceLanguageBoundary } from "../i18n/client";
import { ProjectSettingsSheet } from "../ProjectSettingsSheet";
import { DEFAULT_GUARD_PATHS, settingsTarget, type ProjectSettings, type ProjectSettingsPatch, type ProjectSettingsWrite } from "../projectSettings";
import type { KitAgent, KitPiece } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import { memorySceneProject } from "./MemoryScene";
import { REFERENCE_FOLDS, sidebarScene } from "./sceneData";

export function ProjectSettingsScene({ params }: { params: URLSearchParams }) {
  const theme = params.get("theme") === "light" ? "light" : "dark";
  const content = params.get("content") === "long" ? "long" : "reference";
  const scale = Number(params.get("scale") ?? "1");
  const state = params.get("state") ?? "connections";
  const lang = params.get("lang") ?? "ko";
  if (!["en", "ko", "ja", "zh-CN"].includes(lang)) throw new Error(`Unknown project settings language ${lang}`);
  const scene = useMemo(() => {
    const fixture = sidebarScene(content, REFERENCE_FOLDS, Date.now(), "one");
    if (content === "long") {
      const project = fixture.rest.navigator!.workspaces!.find((row) => !row.is_home && row.is_git)!;
      project.label = "프로젝트 설정과 여러 기기에서 이어지는 아주 긴 한국어 작업 이름";
      project.path = "/fixture/projects/프로젝트-설정-검증을-위한-아주-긴-경로/여러-체크아웃을-함께-사용하는-프로젝트";
    }
    return fixture;
  }, [content]);
  const workspace = scene.rest.navigator!.workspaces!.find((row) => !row.is_home && row.is_git)!;
  const target = settingsTarget(workspace);
  const actions = useMemo(() => createActions((event) => {
    if (event.kind === "project_settings_set") {
      const request = event.payload as { request_id: string; patch: ProjectSettingsPatch };
      // A pattern only the core's gitignore reader refuses (a backwards range) answers as the core does.
      if (request.patch.guardrails?.paths.some((path) => path.includes("[z-a]"))) {
        useShellStore.setState((shell) => ({ rest: { ...shell.rest, ui_state: { ...shell.rest?.ui_state, project_settings_writes: { example: { device_id: workspace.device_id, root: workspace.path, request_id: request.request_id, state: "failed", error: "project_settings_path_invalid", patch: request.patch } } } } }));
        return true;
      }
      useShellStore.setState((shell) => {
        const saved = shell.rest?.ui_state?.project_settings?.example;
        // A record that carries guardrails is version 2, as the core writes it.
        const version = request.patch.guardrails || saved?.guardrails ? 2 : 1;
        return { rest: { ...shell.rest, ui_state: { ...shell.rest?.ui_state, project_settings: { example: { device_id: workspace.device_id, root: workspace.path, ...saved, ...request.patch, version } }, project_settings_writes: { example: { device_id: workspace.device_id, root: workspace.path, request_id: request.request_id, state: "saved", error: null } } } } };
      });
    }
    return true;
  }), [workspace]);
  useLayoutEffect(() => {
    const workspaces = scene.rest.navigator!.workspaces!.map((row) => row === workspace ? { ...row, is_git: state !== "guard-folder", default_branch: row.default_branch ?? "main", home_issues: { ...row.home_issues, issues: row.home_issues?.issues ?? [], overflow: row.home_issues?.overflow ?? false, repository: "example/hide" } } : row);
    const guard = GUARD_STATES[state];
    const receipt: Record<string, ProjectSettingsWrite & { device_id: string; root: string }> = state === "pending" || state === "failed" ? { example: { device_id: workspace.device_id, root: workspace.path, request_id: "scene-intent", state, error: state === "failed" ? "project_settings_save_failed" : null, patch: { issue_source: "local" } } }
      : state === "guard-failed" ? { example: { device_id: workspace.device_id, root: workspace.path, request_id: "scene-intent", state: "failed", error: "project_settings_policy_failed", patch: { guardrails: { ...guard!, git_commands: true } } } } : {};
    const settings: Record<string, ProjectSettings> = guard ? { example: { version: 2, device_id: workspace.device_id, root: workspace.path, guardrails: guard } } : {};
    // The coverage line names the agents of this machine the guardrails do not reach (B25).
    const devices = scene.rest.navigator!.devices!.map((device) => device.id === workspace.device_id && guard ? { ...device, kit: { unavailable: null, busy: false, components: [], agents: sceneAgents(state === "guard-uncovered"), offers_reinstall: false, shares_account_with: null } } : device);
    const rest = { ...scene.rest, navigator: { ...scene.rest.navigator, workspaces, devices }, ui_state: { ...scene.rest.ui_state, interface_language: lang as "en" | "ko" | "ja" | "zh-CN", project_settings: settings, project_settings_writes: receipt } };
    useShellStore.setState({ rest, agents: scene.agents, connection: state === "unavailable" ? "gone" : "live", memory: { project: { ...memorySceneProject(state === "memory-off" ? "off" : "empty"), workspace_id: workspace.id } } });
    useUiStore.setState({ overlay: "project_settings", projectSettingsTarget: target });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", String(scale));
  }, [scene, workspace, theme, scale, lang, state]);
  const page = state === "work" || state.startsWith("guard") ? "work" : state.startsWith("memory") ? "memory" : "connections";
  return <TooltipProvider><InterfaceLanguageBoundary /><div className="h-screen w-full" data-gallery-scene="project-settings" data-scene-ready="true"><ProjectSettingsSheet actions={actions} initialPage={page} /></div></TooltipProvider>;
}

const ALL_ON = { worktree_folder: true, base_branch: true, agent_merge: false, git_commands: true, files: true, paths: DEFAULT_GUARD_PATHS };
const ALL_OFF = { worktree_folder: false, base_branch: false, agent_merge: false, git_commands: false, files: false, paths: DEFAULT_GUARD_PATHS };

/** The Working style states that draw the Guardrails group, by the protections they hold. */
const GUARD_STATES: Record<string, ProjectSettings["guardrails"]> = {
  guard: ALL_ON,
  "guard-off": ALL_OFF,
  "guard-folder": { ...ALL_OFF, files: true },
  "guard-uncovered": ALL_ON,
  "guard-failed": { ...ALL_ON, git_commands: false },
};

const installed: KitPiece = { state: "installed", reason: null, location: null };

/** This machine's agents: every hook in place, or Codex's untrusted and Grok switched off. */
function sceneAgents(uncovered: boolean): KitAgent[] {
  const agent = (id: string, label: string, hook: KitPiece = installed, enabled = true): KitAgent => ({ id, label, availability: "available", enabled, chosen: true, skill: installed, hook, doc_url: "" });
  return [
    agent("claude-code", "Claude Code"),
    agent("codex", "Codex", uncovered ? { state: "failed", reason: "Codex has not trusted Hide's hook", location: null } : installed),
    agent("grok", "Grok", installed, !uncovered),
  ];
}
