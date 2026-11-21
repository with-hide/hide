// Settings › Agents over invented wire values, for review captures: every
// installed agent on one line, a broken one with its one problem line, Codex
// on 직접 입력, Grok with the 일부 기능 chip and Pi with no permission
// question; `state=definitions` adds the agent files under ~/.hide/agents
// that Hide left out, as the development switch shows them. The page takes the window's width up to the sheet's, so a narrow
// window draws it as a phone or a narrow sheet would. Which pieces
// are broken and what each start runs is the core's and is tested there.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { InterfaceLanguageBoundary } from "../i18n/client";
import { AgentsTab } from "../settings/AgentsTab";
import type { KitAgent, KitFeatureId, KitPiece } from "../snapshot";
import { useShellStore } from "../store";

const FEATURES: KitFeatureId[] = ["skill", "guidance", "letters", "bell", "memory", "sleep", "fork", "start", "titles"];
const ok: KitPiece = { state: "installed", why: null, location: null };

const agent = (id: string, label: string, over: Partial<KitAgent> = {}): KitAgent => ({
  id,
  label,
  availability: "available",
  enabled: true,
  chosen: true,
  skill: ok,
  hook: ok,
  partial: false,
  features: FEATURES.map((feature) => ({ id: feature, supported: true })),
  sessions: 0,
  doc_url: "",
  ...over,
});

function agents(state: string): KitAgent[] {
  const broken = state === "broken";
  return [
    agent("claude-code", "Claude Code", { sessions: 12 }),
    agent("codex", "Codex", { sessions: 3 }),
    agent("grok", "Grok", { partial: true, sessions: null, features: FEATURES.map((feature) => ({ id: feature, supported: !["letters", "memory"].includes(feature) })) }),
    agent("opencode", "OpenCode", broken ? { hook: { state: "removed", why: { cause: "taken_out" }, location: "/Users/me/.config/opencode/plugins/hide.js" } } : {}),
    agent("pi", "Pi", broken ? { hook: { state: "failed", why: { cause: "not_changed" }, location: "/Users/me/.pi/agent/extensions/hide.ts" } } : {}),
    agent("omp", "omp", { availability: "not_installed", enabled: false, chosen: false }),
    agent("antigravity", "Antigravity", { availability: "not_installed", enabled: false, chosen: false }),
  ];
}

const LEFT_OUT = {
  rejected: [
    { file: "mypi.toml", reason: "mypi.toml: based_on foo is not a built-in agent" },
    { file: "tool.toml", reason: "tool.toml: program pi is taken by pi" },
  ],
  unreadable: null,
};

export function AgentsSettingsScene({ params }: { params: URLSearchParams }) {
  const theme = params.get("theme") === "light" ? "light" : "dark";
  const state = params.get("state") ?? "healthy";
  if (state !== "healthy" && state !== "broken" && state !== "definitions") throw new Error(`Unknown Agents settings scene state ${state}`);
  const lang = params.get("lang") ?? "ko";
  if (!["en", "ko", "ja", "zh-CN"].includes(lang)) throw new Error(`Unknown Agents settings language ${lang}`);
  const actions = useMemo(() => createActions(() => true), []);
  useLayoutEffect(() => {
    useShellStore.setState({
      connection: "live",
      rest: {
        status: state === "definitions" ? { agent_definitions: LEFT_OUT } : {},
        navigator: {
          siblings_shown: 5,
          focused_device_id: "local",
          devices: [{ id: "local", kind: "local", label: null, machine_name: "Mac", state: "local", kit: { unavailable: null, busy: false, components: [{ id: "cli", state: "installed" }], agents: agents(state), offers_reinstall: false, shares_account_with: null } }],
        },
        ui_state: {
          interface_language: lang,
          agent_permissions: { codex: { mode: "custom", text: "--full-auto", options: ["--full-auto"] } },
          device_agent_permissions: {},
        },
      },
    } as never);
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
  }, [state, theme, lang]);
  return (
    <TooltipProvider>
      <InterfaceLanguageBoundary />
      <div
        className="w-full max-w-[calc(var(--size-settings-sheet-w)-var(--size-settings-nav-w))] bg-background px-lg py-lg"
        data-gallery-scene="agents-settings"
        data-scene-ready="true"
      >
        <AgentsTab actions={actions} />
      </div>
    </TooltipProvider>
  );
}
