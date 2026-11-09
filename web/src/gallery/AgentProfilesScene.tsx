// Agent profiles over invented values, drawn before the screens are wired
// (PRD agent-profiles B1-B4, B10, B14, B15, B26-B28, B30): Settings › Agents
// with each agent's profiles unfolded under its row and edited in a dialog,
// the add form for each way an agent keeps accounts, every row state, the
// start panel's profile menu, a project's Agent accounts and the two delete
// outcomes. Names, paths and numbers are invented; nothing is sent.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { AgentPickerView } from "../components/agent-picker";
import {
  InlineProfiles,
  ProfileDeleteDialog,
  ProfileFormDialog,
  ProfilePicker,
  ProfilesMachineNote,
  ProjectAgentAccounts,
  type AgentProfilesView,
  type ProfileDraft,
  type ProfileRefusal,
  type ProfileView,
  type ProfilesMachine,
} from "../components/agent-profiles";
import { Group } from "../components/settings-rows";
import { Button } from "../components/ui/button";
import { Textarea } from "../components/ui/textarea";
import { ToggleGroup, ToggleGroupItem } from "../components/ui/toggle-group";
import { TooltipProvider } from "../components/ui/tooltip";
import { InterfaceLanguageBoundary, useInterfaceTranslation } from "../i18n/client";
import { AgentRow } from "../settings/AgentRow";
import type { Device, KitAgent, KitFeatureId, KitPiece } from "../snapshot";
import { useShellStore } from "../store";

export const AGENT_PROFILES_STATES = [
  "list-a",
  "list-a-edit",
  "form-folder",
  "form-name",
  "form-none",
  "form-wrapper",
  "form-refused",
  "states",
  "start",
  "project",
  "delete-refused",
  "delete-confirm",
] as const;
type State = (typeof AGENT_PROFILES_STATES)[number];

const FEATURES: KitFeatureId[] = ["skill", "guidance", "letters", "bell", "memory", "sleep", "fork", "start", "titles"];
const ok: KitPiece = { state: "installed", why: null, location: null };
const kit = (id: string, label: string, sessions = 0): KitAgent => ({ id, label, availability: "available", enabled: true, chosen: true, skill: ok, hook: ok, partial: false, features: FEATURES.map((feature) => ({ id: feature, supported: true })), sessions, doc_url: "" });

const KIT: KitAgent[] = [kit("claude-code", "Claude Code", 12), kit("codex", "Codex", 3), kit("omp", "omp", 1), kit("antigravity", "Antigravity")];

const base: Omit<ProfileView, "id" | "name"> = { builtIn: false, isDefault: false, account: null, program: null, envKeys: [], install: "installed", login: "signed_in", usageLeft: null, wrapperFolder: false };
const builtIn = (account: string, over: Partial<ProfileView> = {}): ProfileView => ({ ...base, id: "default", name: "", builtIn: true, isDefault: true, account, install: null, ...over });
const profile = (id: string, over: Partial<ProfileView>): ProfileView => ({ ...base, id, name: id, ...over });

function fixtures(long: boolean) {
  const work = long ? "회사-고객사-프로젝트-전용-계정" : "work";
  const workPath = long ? "~/accounts/회사-고객사-프로젝트-전용-계정-폴더/claude" : "~/accounts/claude-work";
  const claude: AgentProfilesView = {
    agent: "claude-code",
    label: "Claude Code",
    way: "folder",
    program: "claude",
    profiles: [
      builtIn("~/.claude", { usageLeft: 40, isDefault: false }),
      profile("work", { name: work, account: workPath, usageLeft: 72, isDefault: true }),
      profile("side", { account: "~/accounts/claude-side", login: "signed_out" }),
      profile("lab", { account: "~/accounts/claude-lab", install: "failed", login: null }),
      profile("gateway", { program: "claude-gateway", envKeys: ["ANTHROPIC_BASE_URL", "MAX_THINKING_TOKENS"], install: null, login: null, usageLeft: "shared" }),
    ],
  };
  const codex: AgentProfilesView = { agent: "codex", label: "Codex", way: "folder", program: "codex", profiles: [builtIn("~/.codex", { usageLeft: 55 })] };
  const omp: AgentProfilesView = { agent: "omp", label: "omp", way: "name", program: "omp", profiles: [builtIn("", { account: null, login: null }), profile("personal", { account: "personal", login: null })] };
  const agy: AgentProfilesView = { agent: "antigravity", label: "Antigravity", way: "none", program: "agy", profiles: [builtIn("~/.gemini", { login: null })] };
  return { claude, codex, omp, agy, views: [claude, codex, omp, agy] };
}


function Scene({ state, long, device }: { state: State; long: boolean; device: Device }) {
  const { t } = useInterfaceTranslation();
  const data = useMemo(() => fixtures(long), [long]);
  const actions = useMemo(() => createActions(() => true), []);
  const agentRow = (agent: KitAgent) => <AgentRow device={device} agent={agent} actions={actions} onAct={() => {}} />;
  const machines = (
    <ToggleGroup type="single" value="local" aria-label={t("agents.machine")} className="mb-lg">
      <ToggleGroupItem value="local">Mac mini</ToggleGroupItem>
      <ToggleGroupItem value="win-box">win-box</ToggleGroupItem>
    </ToggleGroup>
  );
  const list = (machine: ProfilesMachine = "live") => (
    <Group title={t("agents.installedCount", { count: KIT.length })}>
      {KIT.map((agent, index) => (
        <div key={agent.id} data-agent-with-profiles={agent.id}>
          {agentRow(agent)}
          <InlineProfiles view={data.views[index]!} machine={machine} />
        </div>
      ))}
    </Group>
  );
  const claude = data.claude;
  const work = claude.profiles[1]!;
  const workDraft: ProfileDraft = { name: work.name, account: work.account ?? "", program: "", env: [{ key: "MAX_THINKING_TOKENS", value: "8000" }] };

  switch (state) {
    case "list-a":
      return <>{machines}{list()}</>;
    case "list-a-edit":
      return <>{machines}{list()}<ProfileFormDialog open view={claude} mode="edit" profile={work} draft={workDraft} /></>;
    case "form-folder":
      return <>{list()}<ProfileFormDialog open view={claude} mode="add" draft={{ name: long ? "회사-고객사-프로젝트-전용-계정" : "client", account: long ? "~/accounts/회사-고객사-프로젝트-전용-계정-폴더/claude" : "~/accounts/claude-client", program: "", env: [] }} /></>;
    case "form-name":
      return <>{list()}<ProfileFormDialog open view={data.omp} mode="add" draft={{ name: "work", account: "work", program: "", env: [] }} /></>;
    case "form-none":
      return <>{list()}<ProfileFormDialog open view={data.agy} mode="add" draft={{ name: "gateway", account: "", program: "agy-gateway", env: [] }} /></>;
    case "form-wrapper":
      return <>{list()}<ProfileFormDialog open view={claude} mode="add" draft={{ name: "gateway", account: "", program: "~/bin/claude-gateway", env: [{ key: "ANTHROPIC_BASE_URL", value: "https://gateway.example.test" }, { key: "MAX_THINKING_TOKENS", value: "8000" }] }} /></>;
    case "form-refused": {
      const refusals: ProfileRefusal[] = [
        { field: "account", text: t("agentProfiles.refuse.defaultLocation") },
        { field: "env", envKey: "CLAUDE_CONFIG_DIR", text: t("agentProfiles.refuse.accountVar", { key: "CLAUDE_CONFIG_DIR" }) },
        { field: "env", envKey: "HOME", text: t("agentProfiles.refuse.reserved", { key: "HOME" }) },
      ];
      return <>{list()}<ProfileFormDialog open view={claude} mode="add" draft={{ name: "mine", account: "~/.claude", program: "", env: [{ key: "CLAUDE_CONFIG_DIR", value: "~/accounts/x" }, { key: "HOME", value: "/tmp/x" }] }} refusals={refusals} /></>;
    }
    case "states": {
      const rows: AgentProfilesView = {
        ...claude,
        profiles: [
          builtIn("~/.claude", { usageLeft: 40, isDefault: false }),
          profile("work", { account: "~/accounts/claude-work", usageLeft: 72, isDefault: true }),
          profile("new", { account: "~/accounts/claude-new", install: "installing", login: null }),
          profile("lab", { account: "~/accounts/claude-lab", install: "failed", login: null }),
          profile("side", { account: "~/accounts/claude-side", login: "signed_out" }),
          profile("locked", { account: "~/accounts/claude-locked", login: "unknown" }),
          profile("gateway", { program: "claude-gateway", install: null, login: null, wrapperFolder: true }),
          profile("old", { account: "~/accounts/claude-old", install: "remove_failed", login: null }),
          profile("relay", { program: "claude-relay", envKeys: ["ANTHROPIC_BASE_URL"], install: null, login: null, usageLeft: "shared" }),
        ],
      };
      const remote: AgentProfilesView = { ...claude, profiles: [builtIn("C:/Users/me/.claude", { usageLeft: null, isDefault: false }), profile("work", { account: "C:/Users/me/accounts/claude-work", isDefault: true })] };
      return (
        <>
          <Group title="Mac mini">
            <div>{agentRow(KIT[0]!)}<InlineProfiles view={rows} /></div>
          </Group>
          <Group title="win-box" note={<ProfilesMachineNote machine="offline" device="win-box" />}>
            <div>{agentRow(KIT[0]!)}<InlineProfiles view={remote} machine="offline" /></div>
          </Group>
          <Group title="old-box" note={<ProfilesMachineNote machine="unsupported" device="old-box" />}>
            <div>{agentRow(KIT[0]!)}<InlineProfiles view={claude} machine="unsupported" /></div>
          </Group>
        </>
      );
    }
    case "start": {
      return (
        <div className="flex w-(--size-search-sheet-w) max-w-full flex-col overflow-hidden rounded-lg border border-border bg-dialog text-body text-popover-foreground [--accent:var(--dialog-raised)] [--secondary:var(--dialog-raised)] shadow-lg" data-start-panel-mock="true">
          <div className="px-sm pt-sm text-caption text-subtle-foreground">blog › main</div>
          <Textarea bare className="field-sizing-content min-h-(--size-control-lg) px-md pt-xs pb-sm" placeholder={t("shell.instructionPrompt")} />
          <div className="flex min-w-0 items-center gap-xs px-sm pt-xs pb-sm">
            <AgentPickerView variant="ghost" value={{ kind: "claude", model: null }} catalog={{ state: "ready", models: [] }} onKind={() => {}} onModel={() => {}}
              kindNote={<ProfilePicker view={claude} value="side" projectDefault="side" defaultOpen />} />
            <span className="flex-1" />
            <Button>{t("common.start")}<span aria-hidden="true" className="opacity-(--opacity-dimmed)">⏎</span></Button>
          </div>
        </div>
      );
    }
    case "project":
      return (
        <ProjectAgentAccounts rows={[
          { view: claude, chosen: "side" },
          { view: data.omp, chosen: "personal" },
          { view: { ...data.codex, profiles: [...data.codex.profiles, profile("team", { account: "~/accounts/codex-team" })] }, chosen: null },
          { view: data.agy, chosen: null },
        ]} />
      );
    case "delete-refused":
      return <>{list()}<ProfileDeleteDialog open profile="work" location="~/accounts/claude-work" uses={[]} inUse={[
        { id: "agent-31", title: long ? "로그인 화면의 예외 상황을 모두 찾아서 정리하기" : "fix-bell", place: "herdr-ide", asleep: false },
        { id: "agent-44", title: "usage-store", place: "blog", asleep: true },
      ]} /></>;
    case "delete-confirm":
      return <>{list()}<ProfileDeleteDialog open profile="work" location={long ? "~/accounts/회사-고객사-프로젝트-전용-계정-폴더/claude" : "~/accounts/claude-work"} inUse={[]} uses={[
        ...(long ? ["개인 블로그와 여러 기기에서 이어지는 아주 긴 프로젝트 이름", "side-app"] : ["blog", "side-app"]).map((name) => ({ kind: "project" as const, name })),
        { kind: "candidates", count: 2 },
      ]} /></>;
  }
}

export function AgentProfilesScene({ params }: { params: URLSearchParams }) {
  const theme = params.get("theme") === "light" ? "light" : "dark";
  const state = (params.get("state") ?? "list-a") as State;
  if (!(AGENT_PROFILES_STATES as readonly string[]).includes(state)) throw new Error(`Unknown agent profiles scene state ${state}`);
  const long = params.get("content") === "long";
  const lang = params.get("lang") ?? "ko";
  if (!["en", "ko", "ja", "zh-CN"].includes(lang)) throw new Error(`Unknown agent profiles language ${lang}`);
  const scale = Number(params.get("scale") ?? "1");
  const device = useMemo<Device>(() => ({ id: "local", kind: "local", label: null, machine_name: "Mac mini", state: "local", kit: { unavailable: null, busy: false, components: [{ id: "cli", state: "installed" }], agents: KIT, offers_reinstall: false, shares_account_with: null } }) as never, []);
  useLayoutEffect(() => {
    useShellStore.setState({
      connection: "live",
      rest: {
        status: {},
        navigator: { siblings_shown: 5, focused_device_id: "local", devices: [device] },
        ui_state: { interface_language: lang, agent_permissions: {}, device_agent_permissions: {} },
      },
    } as never);
    const root = document.documentElement;
    root.classList.toggle("dark", theme === "dark");
    root.classList.toggle("light", theme === "light");
    root.style.setProperty("--interface-scale", String(scale));
  }, [device, theme, lang, scale]);
  return (
    <TooltipProvider>
      <InterfaceLanguageBoundary />
      <div className="min-h-screen bg-background text-foreground">
        <div className="w-full max-w-[calc(var(--size-settings-sheet-w)-var(--size-settings-nav-w))] px-lg py-lg" data-gallery-scene="agent-profiles" data-agent-profiles-state={state} data-scene-ready="true">
          <Scene state={state} long={long} device={device} />
        </div>
      </div>
    </TooltipProvider>
  );
}
