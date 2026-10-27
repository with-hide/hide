// The first run on invented data (PRD first-run-onboarding B42): the shell's
// real Sidebar and Home behind the real FirstRunWizard, start panel or
// Settings sheet, over a synthetic This Mac with the seven agents and no
// daemon. A desktop host stands in for the Electron bridge, so the steps and
// the notification lines draw as the desktop app draws them, on the system
// the query names; the scene's state is the step the core would keep and
// what the host would answer. One scene fills one document, because the
// stores it seeds are the app's singletons.

import { useLayoutEffect, useMemo, useState } from "react";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { FirstRunWizard, useSetupWalk } from "../FirstRunWizard";
import { HerdrGate } from "../HerdrTakeover";
import { herdrGate } from "../herdrTakeoverRules";
import type { HostBridge, NotificationPermission, UpdateStatus } from "../host";
import type { MobileState } from "../mobileSettings";
import { clientI18n } from "../i18n/translator";
import { OverviewPage } from "../Overview";
import { SettingsGate } from "../SettingsSheet";
import { SETTINGS_GROUPS, type SettingsTab } from "../settings";
import { Sidebar } from "../sidebar";
import type { AiProvider, HerdrProfile, HerdrTakeover, KitAgent, KitPiece, SnapshotRest } from "../snapshot";
import { StartPanelHost } from "../StartPanel";
import { useStartPanel } from "../startDraft";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import { EMPTY_GALLERY_SCOPE } from "./agentScopes";
import { checkout, REFERENCE_FOLDS, workspace } from "./sceneData";

/** Every state the scene draws; each names the step, what is open and what the host answers. */

/** The command a refused pass names on each system to stop the app's background service, as the kit spells it there. */
const REFUSED_STOP: Record<OnboardingSystem, string> = {
  mac: "HIDE_STATE_DIR='/Volumes/Team/hide' '/Applications/hide.app/Contents/Resources/hide' stop",
  linux: "HIDE_STATE_DIR='/srv/team/hide' '/opt/hide/resources/hide' stop",
  windows: "$env:HIDE_STATE_DIR = 'D:\\hide'; & 'C:\\Users\\me\\.hide\\runtime\\0.4.1\\hide.exe' stop; Remove-Item Env:HIDE_STATE_DIR",
};

export const ONBOARDING_STATES = [
  "agents",
  "agents-changes",
  "agents-failed",
  "agents-none",
  "agents-refused",
  "agents-setup",
  "agents-update",
  "devices",
  "devices-loading",
  "devices-failed",
  "devices-disabled",
  "devices-partial",
  "devices-long32",
  "notifications",
  "notifications-asking",
  "notifications-on",
  "notifications-blocked",
  "notifications-unreadable",
  "notifications-no-service",
  "project",
  "project-add",
  "start-panel",
  "start-panel-signed-out",
  "home-no-project",
  "home-no-agent",
  "settings-on",
  "settings-off",
  "settings-blocked",
  "settings-done-only",
  "settings-both",
  "settings-refused",
  "settings-update",
  "herdr-moving",
  "herdr-failed",
  "herdr-moved",
  "herdr-moved-behind",
  "herdr-restart",
  "herdr-look",
  "gate-newer",
  "gate-mismatch",
] as const;

export type OnboardingState = (typeof ONBOARDING_STATES)[number];
export type OnboardingSystem = "mac" | "windows" | "linux";

export type OnboardingSceneParams = {
  theme: "light" | "dark";
  state: OnboardingState;
  system: OnboardingSystem;
  language: "en" | "ko" | "ja" | "zh-CN";
  /** The interface text scale Appearance's font size sets (`--interface-scale`); the largest is 17/13 (B41). */
  scale: number;
  /** The Settings page a `settings-*` state opens (PRD settings-left-nav); Notifications unless the query names another. */
  page?: SettingsTab;
  /** This app's update as the host reports it (PRD auto-update), for About's row and the Help dot; none when absent. */
  update?: UpdateStatus;
};

/** `update=<state>` in the query: the states About's update row draws (PRD auto-update D-20). */
const SCENE_UPDATES: Record<string, UpdateStatus> = {
  idle: { phase: { kind: "idle", checkedAt: Date.now() - 12 * 60_000 }, prerelease: false, installOnQuit: true },
  downloading: { phase: { kind: "downloading", version: "0.5.0", percent: 42 }, prerelease: false, installOnQuit: true },
  ready: { phase: { kind: "ready", version: "0.5.0" }, prerelease: false, installOnQuit: true },
  "ready-linux": { phase: { kind: "ready", version: "0.5.0" }, prerelease: true, installOnQuit: false },
  failed: { phase: { kind: "failed", stage: "download" }, prerelease: false, installOnQuit: true },
  location: { phase: { kind: "off", reason: "location" }, prerelease: false, installOnQuit: true },
};

export function onboardingSceneParams(params: URLSearchParams): OnboardingSceneParams {
  const state = params.get("state") ?? "agents";
  if (!(ONBOARDING_STATES as readonly string[]).includes(state)) throw new Error(`Unknown onboarding scene state ${state}`);
  const system = params.get("system") ?? "mac";
  if (system !== "mac" && system !== "windows" && system !== "linux") throw new Error(`Unknown onboarding scene system ${system}`);
  // The long content is the Korean copy, the longest of the four languages (B41).
  const requestedLanguage = params.get("lang");
  const language = requestedLanguage === "ko" || requestedLanguage === "ja" || requestedLanguage === "zh-CN" ? requestedLanguage : params.get("content") === "long" ? "ko" : "en";
  const scale = Number(params.get("scale") ?? "1");
  if (!(scale > 0)) throw new Error(`Scene scale must be a positive number, got ${params.get("scale")}`);
  const page = params.get("page") ?? "notifications";
  if (!SETTINGS_GROUPS.some((group) => group.tabs.includes(page as SettingsTab)) && page !== "about" && page !== "privacy") throw new Error(`Unknown Settings page ${page}`);
  const update = params.get("update");
  if (update !== null && !(update in SCENE_UPDATES)) throw new Error(`Unknown update state ${update}`);
  return { theme: params.get("theme") === "light" ? "light" : "dark", state: state as OnboardingState, system, language, scale, page: page as SettingsTab, update: update === null ? undefined : SCENE_UPDATES[update] };
}

const HOME = "/Users/me";

/** Phone access off, no phone paired, push only while the app is closed: the daemon's `mobile` frame on a fresh Mac. */
const SCENE_MOBILE: MobileState = {
  enabled: false,
  exposure: "off",
  checklist: null,
  download_url: "https://example.test/tailscale",
  admin_url: "https://example.test/admin",
  foreign_target: null,
  failure: null,
  url: null,
  qr: null,
  code_expires_at_ms: null,
  phones: [],
  max_phones: 4,
  push_mode: "app_closed",
  now_ms: 0,
};
const PLATFORM: Record<OnboardingSystem, string> = { mac: "darwin", windows: "win32", linux: "linux" };

/** The kit's agents in its order, each as KitAgentSnapshot reports it on a Mac whose kit has not run. */
function kitAgents(state: OnboardingState, system: OnboardingSystem): KitAgent[] {
  const piece = (location: string | null, failed: string | null = null): KitPiece => ({ state: failed ? "failed" : "off", why: failed ? { cause: failed } : null, location });
  const shared = `${HOME}/.agents/skills/hide/SKILL.md`;
  const none = state === "agents-none";
  const row = (id: string, label: string, installed: boolean, pieces: { hook?: string; skill: string; herdr: string }, extra: Partial<KitAgent> = {}): KitAgent => ({
    id,
    label,
    availability: none || !installed ? "not_installed" : "available",
    enabled: false,
    chosen: false,
    skill: piece(pieces.skill),
    hook: pieces.hook ? piece(pieces.hook) : null,
    herdr: piece(pieces.herdr),
    doc_url: "https://example.test",
    ...extra,
  });
  // Off macOS, OpenCode is installed but Hide cannot attach to it there yet (B49).
  const pc = system !== "mac";
  // A Claude Code older than Hide's minimum, installed by its own installer, and an omp whose install Hide cannot
  // tell, which gets its install page (docs/agent-hooks.md, Updating an agent that is too old).
  const tooOld = state === "agents-update" || state === "settings-update";
  const refusedOld = state === "settings-update" ? "update_agent" : null;
  const old = (installed: string, minimum: string, path: string, command: string | null, pieces: { hook: string; skill: string; herdr: string }): Partial<KitAgent> =>
    tooOld ? { update: { installed, minimum, path, command }, ...(refusedOld ? { hook: { ...piece(pieces.hook, refusedOld), why: { cause: refusedOld, value: minimum } }, skill: piece(pieces.skill), herdr: piece(pieces.herdr) } : {}) } : {};
  const claude = { hook: `${HOME}/.claude/settings.json`, skill: `${HOME}/.claude/skills/hide/SKILL.md`, herdr: `${HOME}/.claude` };
  const omp = { hook: `${HOME}/.omp/agent/extensions/hide.ts`, skill: shared, herdr: `${HOME}/.omp/agent` };
  return [
    row("claude-code", "Claude Code", true, claude, old("2.1.200", "2.1.278", `${HOME}/.local/share/claude/versions/2.1.200`, "claude update", claude)),
    row("codex", "Codex", true, { hook: `${HOME}/.codex/hooks.json`, skill: shared, herdr: `${HOME}/.codex` }, state === "agents-failed" ? { enabled: true, chosen: true, hook: piece(`${HOME}/.codex/hooks.json`, "not_changed") } : {}),
    row("grok", "Grok", false, { hook: `${HOME}/.grok/hooks/hide.json`, skill: shared, herdr: `${HOME}/.grok` }),
    row("opencode", "OpenCode", true, { hook: `${HOME}/.config/opencode/plugins/hide.js`, skill: shared, herdr: `${HOME}/.config/opencode` }, pc ? { availability: "unsupported_system" } : {}),
    row("pi", "Pi", true, { hook: `${HOME}/.pi/agent/extensions/hide.ts`, skill: shared, herdr: `${HOME}/.pi/agent` }),
    row("omp", "omp", !pc, omp, state === "settings-update" ? old("18.2.1", "18.7.0", `${HOME}/src/oh-my-pi/bin/omp`, null, omp) : {}),
    row("antigravity", "Antigravity", false, { hook: `${HOME}/.gemini/config/hooks.json`, skill: `${HOME}/.gemini/config/skills/hide/SKILL.md`, herdr: `${HOME}/.gemini/config` }),
  ];
}

/** Claude Code signed in, Codex signed out, Pi not read yet: the three lines a tile can say (B8). */
function providers(): AiProvider[] {
  const provider = (id: string, label: string, agent: string, state: string): AiProvider => ({ id, label, agent, state, headline: "", message: null, installed: true, selectable: state === "ready", retry_at_ms: null, model: "", models: id === "claude" ? ["opus", "sonnet"] : ["gpt-6-astra"], models_fixed: false, cli_default: true, models_unavailable_reason: null });
  return [provider("claude", "Claude Code", "claude-code", "ready"), provider("codex", "Codex", "codex", "needs_login")];
}

function permissionOf(state: OnboardingState, system: OnboardingSystem): NotificationPermission {
  if (state === "notifications-no-service") return "no_service";
  if (system !== "mac") return "unsupported";
  if (state === "notifications-on" || state === "settings-on" || state === "settings-done-only" || state === "settings-both") return "granted";
  if (state === "notifications-blocked" || state === "settings-blocked") return "denied";
  if (state === "notifications-unreadable") return "unknown";
  return "not_determined";
}

/** The desktop host as far as these screens ask it; nothing is shown or opened. */
function sceneHost(system: OnboardingSystem, permission: NotificationPermission, update: UpdateStatus | undefined): HostBridge {
  const none = () => () => undefined;
  return {
    kind: "electron",
    platform: PLATFORM[system],
    onCommand: none,
    reportBindings: () => undefined,
    reportLanguage: () => undefined,
    revealPath: () => undefined,
    quit: () => undefined,
    openPractice: () => undefined,
    pickFolder: async () => null,
    probePaths: async (paths) => paths.map(() => null),
    openPath: () => undefined,
    notify: () => undefined,
    onNotificationOpen: none,
    withdrawNotifications: () => undefined,
    notificationPermission: async () => permission,
    onWindowChrome: () => () => undefined, editorAvailable: async () => false, openInEditor: () => undefined, restartHerdr: async () => "unavailable", openNotificationSettings: () => undefined,
    reportBadge: () => undefined,
    browser: { sync: () => undefined, endCycle: () => undefined, capture: async () => null, command: () => undefined, onEvent: none },
    updates: {
      onStatus: (listener) => {
        if (update) listener(update);
        return () => undefined;
      },
      check: () => undefined,
      restart: () => undefined,
      setPrerelease: () => undefined,
    },
  };
}

/**
 * The operator's own Herdr as the core publishes it for the `herdr-*` and `gate-*` states (PRD herdr-takeover, operator
 * decision 2026-10-23): the move the core started on its own, its failure, its result with the terminal's `herdr`, a
 * restart, a look that could not read the server, a newer Herdr, and a mismatch the core offers nothing for.
 */
function sceneHerdr(state: OnboardingState): { herdr: Record<string, unknown>; herdr_takeover?: HerdrTakeover; herdr_versions: Record<string, unknown> } {
  const offer = (patch: Partial<HerdrTakeover>): HerdrTakeover => ({ kind: "move", stage: "moving", server_version: "0.8.2", version: "0.9.5", same_protocol: false, failure: null, path_herdr: null, ...patch });
  const older = { state: "protocol_mismatch", received_version: "0.8.2", received_protocol: 20, expected_protocol: 22 };
  const versions = { hide: "0.9.5", path_herdr: { version: "0.8.2", behind: true, update_command: "sudo install -m 755 '/Applications/hide.app/Contents/Resources/herdr' '/usr/local/bin/herdr'" } };
  const connected = { state: "connected", received_version: "0.9.5", received_protocol: 22, expected_protocol: 22 };
  switch (state) {
    case "herdr-moving": return { herdr: older, herdr_takeover: offer({}), herdr_versions: versions };
    case "herdr-failed": return { herdr: older, herdr_takeover: offer({ stage: "failed", failure: "refused" }), herdr_versions: versions };
    case "herdr-moved": return { herdr: connected, herdr_takeover: offer({ stage: "moved", path_herdr: { stage: "updated", version: "0.9.5" } }), herdr_versions: versions };
    case "herdr-moved-behind": return { herdr: connected, herdr_takeover: offer({ stage: "moved", path_herdr: { stage: "behind", version: "0.8.2" } }), herdr_versions: versions };
    case "herdr-restart": return { herdr: { ...older, received_version: "0.6.2", received_protocol: 10 }, herdr_takeover: offer({ kind: "restart", stage: "offered", server_version: "0.6.2" }), herdr_versions: versions };
    case "herdr-look": return { herdr: { state: "malformed" }, herdr_takeover: offer({ kind: "look", stage: "failed", failure: "unreadable", server_version: "", version: "" }), herdr_versions: versions };
    case "gate-newer": return { herdr: { state: "protocol_mismatch", received_version: "0.10.0", received_protocol: 23, expected_protocol: 22 }, herdr_takeover: offer({ kind: "update_hide", stage: "offered", server_version: "0.10.0" }), herdr_versions: versions };
    case "gate-mismatch": return { herdr: older, herdr_versions: versions };
    default: return { herdr: connected, herdr_versions: versions };
  }
}

function sceneRest(state: OnboardingState, system: OnboardingSystem): SnapshotRest {
  const now = Math.floor(Date.now() / 1000);
  const projects = state.startsWith("project") || state.startsWith("agents") || state.startsWith("devices") || state.startsWith("notifications") || state.startsWith("herdr") || state === "home-no-project" ? [] : [
    workspace("my-app", { path: "/work/my-app", checkouts: [checkout({ id: "my-app:main", workspace: "my-app", branch: "main", primary: true, age: 3600 }, now)] }, REFERENCE_FOLDS),
  ];
  // Help's walk is the page's own, over a first run that ended (B45).
  const step = state === "agents-setup" ? "done" : state.startsWith("herdr") ? "herdr" : state.startsWith("agents") ? "agents" : state.startsWith("devices") ? "devices" : state.startsWith("notifications") ? "notifications" : state.startsWith("project") ? "project" : "done";
  // The switch is on once Turn on was pressed; macOS's answer is the host's (`permissionOf`).
  const enabled = (state.startsWith("notifications-") && state !== "notifications-no-service") || state === "settings-on" || state === "settings-blocked" || state === "settings-both";
  // A first pass that stopped before changing anything: a state folder HIDE_STATE_DIR chose that other
  // accounts can change, read as each system spells the folder at fault; nothing was judged, so no agent is listed.
  const refused = state === "agents-refused" || state === "settings-refused";
  const agents = refused ? [] : kitAgents(state, system).map((agent) => (step === "agents" || agent.availability !== "available" ? agent : { ...agent, enabled: true, chosen: true }));
  const kit = refused
    ? { agents, components: [], busy: false, refused: { cause: "unsafe", path: system === "windows" ? "D:\\" : system === "linux" ? "/srv/team" : "/Volumes/Team", variable: "HIDE_STATE_DIR", stop: REFUSED_STOP[system] } }
    : { agents, components: [{ id: "cli", label: "hide", state: "off", why: null, location: system === "windows" ? `${HOME}/.local/bin/hide.cmd` : `${HOME}/.local/bin/hide` }], busy: false };
  const profile = (id: string, label: string, patch: Partial<HerdrProfile> = {}): HerdrProfile => ({ id, label, target: `${id}@work.example`, session: "work", enabled: true, added_as: null, device_id: null, problem: null, connect_state: "saved", connect_refusal: null, ...patch });
  const savedProfiles = state === "devices-long32" ? Array.from({ length: 32 }, (_, index) => profile(`research-${index}`, `연구 개발팀의 아주 긴 기기 이름 ${index + 1} · 프로젝트 빌드 서버`, { target: `developer-${index}@very-long-research-hostname.engineering.example.test:2222`, session: "research-team-project-session-with-a-long-name" }))
    : state === "devices-disabled" ? [profile("build", "Build server", { enabled: false, problem: "disabled" }), profile("existing", "Design Mac", { added_as: "Design Mac", device_id: "existing", connect_state: "added" }), profile("invalid", "Unavailable configuration", { problem: "invalid_target" })]
    : state === "devices-partial" ? [profile("build", "Build server", { added_as: "Build server", device_id: "build", connect_state: "connected" }), profile("research", "Research server", { added_as: "Research server", device_id: "research", connect_state: "failed", connect_refusal: null })]
    : [profile("build", "Build server"), profile("research", "Research server", { session: "experiments" })];
  const deviceStep = state.startsWith("devices");
  return {
    revision: 1,
    navigator: {
      agent_scope: EMPTY_GALLERY_SCOPE,
      workspaces: projects,
      inactive_projects: [],
      agents: [],
      devices: [{ agent_scope: EMPTY_GALLERY_SCOPE, id: "local", label: "This Mac", kind: "local", state: "local", message: null, ssh_alias: null, agent_count: 0, test: null, kit }, ...(state === "devices-partial" ? [{ agent_scope: EMPTY_GALLERY_SCOPE, id: "build", label: "Build server", kind: "remote", state: "connected", message: null, ssh_alias: "build", agent_count: 0, test: null, kit }] : [])],
      focused_device_id: "local",
      focused_checkout_id: null,
    },
    ui_state: {
      left_sidebar_visible: true,
      first_run: { version: 1, step },
      first_run_choices: { devices: state === "devices-partial" ? ["build", "research"] : [], project_form: { view: state === "project-add" ? "add" : "" } },
      agent_onboarding: state === "agents-refused" ? null : step === "agents" || step === "herdr" ? "pending" : "done",
      desktop_notifications: enabled,
      desktop_notifications_done: state === "settings-done-only" || state === "settings-both",
      workspace_registrations: [{ id: "home", label: "hide", path: `${HOME}/hide`, device_id: "local", pinned: false, home: true }, ...projects.map((row) => ({ id: row.id, label: row.label, path: row.path, device_id: "local", pinned: false }))],
    },
    status: { ...sceneHerdr(state), background_ai: { enabled: true, provider: "claude", chosen: true, providers: providers(), unavailable_reason: null }, herdr_profiles: { state: state === "devices-loading" ? "loading" : state === "devices-failed" ? "failed" : "ready", error: state === "devices-failed" ? "Saved profiles could not be read" : null, profiles: deviceStep ? savedProfiles : [], truncated: false }, remote: state === "devices-partial" ? [{ target_id: "build", state: "connected" }, { target_id: "research", state: "error", message: "SSH access was refused" }] : [] },
  } as unknown as SnapshotRest;
}

/** The screen that holds Hide, as `App` draws it in place of the shell; here over it, for a crop. */
function SceneGate({ actions }: { actions: ReturnType<typeof createActions> }) {
  const gate = useShellStore((s) => herdrGate(s.rest));
  return gate ? <HerdrGate actions={actions} gate={gate} /> : null;
}

export function OnboardingScene({ theme, state, system, language, scale, page = "notifications", update: requested }: OnboardingSceneParams) {
  // A newer Herdr's wait shows the last check's answer: no Hide made for it yet.
  const update = requested ?? (state === "gate-newer" ? SCENE_UPDATES.idle : undefined);
  // This fixture changes invented snapshots only. It records consent presses,
  // and never dispatches to a daemon, SSH target or native permission service.
  const actions = useMemo(() => createActions((event) => {
    const container = document.querySelector<HTMLElement>('[data-gallery-scene="onboarding"]');
    if (container) container.dataset.sceneEvents = JSON.stringify([...(JSON.parse(container.dataset.sceneEvents ?? "[]") as unknown[]), event]);
    useShellStore.setState((current) => {
      if (!current.rest) return current;
      const rest = structuredClone(current.rest);
      const ui = rest.ui_state!;
      const payload = event.payload as Record<string, unknown>;
      if (event.kind === "first_run_draft") ui.first_run_choices = { ...ui.first_run_choices, ...payload };
      if (event.kind === "first_run_back" || event.kind === "first_run_advance") {
        const steps = rest.status?.herdr_profiles?.profiles.length ? ["agents", "devices", "notifications", "project", "done"] : ["agents", "notifications", "project", "done"];
        const index = steps.indexOf(String(payload.from));
        if (ui.first_run?.step === payload.from && index >= 0) ui.first_run = { version: 1, step: steps[index + (event.kind === "first_run_back" ? -1 : 1)] as NonNullable<typeof ui.first_run>["step"] };
      }
      if (event.kind === "herdr_profile_connect") {
        const row = rest.status?.herdr_profiles?.profiles.find((entry) => entry.id === payload.profile_id);
        if (row) row.connect_state = "connecting";
      }
      return { rest, revision: current.revision + 1 };
    });
    return true;
  }), []);
  // The host and the stores are in place before the first render, since the wizard asks the host which one it is in.
  useMemo(() => {
    window.hideHost = sceneHost(system, permissionOf(state, system), update);
    useShellStore.setState({ connection: "live", rest: sceneRest(state, system), agents: [], revision: 1, mobile: SCENE_MOBILE } as never);
    useUiStore.setState({ screen: { kind: "main" }, overviewOpen: false, mainView: "backlog" } as never);
  }, [state, system, update]);

  // Set once the presses below have landed and a frame has drawn them, so a capture waits for it.
  const [ready, setReady] = useState(false);
  // What a screen shows only after the operator opens or presses something.
  useLayoutEffect(() => {
    setReady(false);
    const settle = () => requestAnimationFrame(() => requestAnimationFrame(() => setReady(true)));
    // A dialog's content mounts in its portal a frame later, so a press waits for its control.
    const click = (selector: string, then?: () => void) => {
      let frames = 60;
      const press = () => {
        const control = document.querySelector(selector) as HTMLElement | null;
        if (!control) return void (frames-- > 0 ? requestAnimationFrame(press) : settle());
        control.click();
        then?.();
        settle();
      };
      press();
    };
    if (state === "agents-setup") useSetupWalk.getState().open();
    if (state.startsWith("settings-")) useUiStore.getState().openSettings(page);
    if (state.startsWith("start-panel")) useStartPanel.getState().open(false, { projectId: "my-app", kind: state === "start-panel" ? "claude" : "codex" });
    // Effects run twice in development, so the toggle is pressed only while it is closed.
    if (state === "agents-changes") click('[data-first-run-what-changes][aria-expanded="false"]');
    // The pass that took the press has landed.
    if (state === "agents-failed") click("[data-onboarding-apply]", () => useShellStore.setState((current) => ({ revision: current.revision + 1 }) as never));
    if (state !== "agents-changes" && state !== "agents-failed") settle();
  }, [state, page]);

  useLayoutEffect(() => {
    void clientI18n.changeLanguage(language);
    document.documentElement.lang = language;
  }, [language]);

  useLayoutEffect(() => {
    const root = document.documentElement;
    root.classList.toggle("dark", theme === "dark");
    root.classList.toggle("light", theme === "light");
  }, [theme]);

  // The shell sets the same property from the stored font size (App.tsx).
  useLayoutEffect(() => {
    const root = document.documentElement.style;
    if (scale === 1) root.removeProperty("--interface-scale");
    else root.setProperty("--interface-scale", String(scale));
  }, [scale]);

  return (
    <TooltipProvider>
      <div className="flex h-full bg-background text-foreground" data-gallery-scene="onboarding" data-scene-state={state} data-scene-system={system} data-scene-ready={ready ? "true" : undefined}>
        <Sidebar actions={actions} />
        <main className="flex min-h-0 min-w-0 flex-1 flex-col">
          <OverviewPage actions={actions} />
        </main>
        <StartPanelHost actions={actions} />
        <SettingsGate actions={actions} />
        {state.startsWith("gate-") ? <SceneGate actions={actions} /> : <FirstRunWizard actions={actions} />}
      </div>
    </TooltipProvider>
  );
}
