// Settings › Hide AI over invented wire values: Runs on on an agent of a
// connected device, connected (`device`) or away (`device_away`), for review
// captures (PRD wsl-device B11, B13). Which rows exist and when one cannot
// answer is the core's and is tested in herdr-core.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { InterfaceLanguageBoundary } from "../i18n/client";
import { HideAiTab } from "../settings/HideAiTab";
import type { AiProvider, BackgroundAi } from "../snapshot";
import { useShellStore } from "../store";

const DEVICE = "wsl-ubuntu";

const row = (id: string, label: string, over: Partial<AiProvider> = {}): AiProvider => ({
  id,
  label,
  agent: id === "claude" ? "claude-code" : id,
  state: "ready",
  headline: "",
  message: null,
  installed: true,
  selectable: true,
  login_checked: true,
  retry_at_ms: null,
  model: id === "claude" ? "sonnet" : "gpt-5.6-luna",
  models: id === "claude" ? ["sonnet", "opus", "haiku"] : ["gpt-5.6-luna"],
  models_fixed: false,
  cli_default: false,
  models_unavailable_reason: null,
  weekly: null,
  past_limit: false,
  ...over,
});

export function HideAiSettingsScene({ params }: { params: URLSearchParams }) {
  const theme = params.get("theme") === "light" ? "light" : "dark";
  const state = params.get("state") ?? "device";
  if (state !== "device" && state !== "device_away") throw new Error(`Unknown Hide AI settings scene state ${state}`);
  const lang = params.get("lang") ?? "ko";
  if (!["en", "ko", "ja", "zh-CN"].includes(lang)) throw new Error(`Unknown Hide AI settings language ${lang}`);
  const actions = useMemo(() => createActions(() => true), []);
  useLayoutEffect(() => {
    const away = state === "device_away";
    const ai: BackgroundAi = {
      enabled: true,
      provider: "claude",
      device: DEVICE,
      chosen: true,
      agent_summary: true,
      usage_limit: 90,
      providers: [
        row("claude", "Claude Code", { state: "needs_login", selectable: false }),
        row("codex", "Codex"),
        away
          ? row("claude", "Claude Code", { device_id: DEVICE, state: "unavailable", message: "device_not_connected", selectable: false })
          : row("claude", "Claude Code", { device_id: DEVICE }),
      ],
      fallback: [{ provider: "codex", device: null, model: "gpt-5.6-luna" }],
      refusal: away ? { provider: "claude", reason: "unavailable", retry_at_ms: null, using: "codex" } : null,
      unavailable_reason: null,
    };
    useShellStore.setState({
      connection: "live",
      rest: {
        status: { background_ai: ai },
        navigator: { devices: [{ id: "local", kind: "local", label: null, machine_name: "Windows PC" }, { id: DEVICE, kind: "remote", label: "WSL Ubuntu", state: away ? "unavailable" : "ready" }] },
        ui_state: { interface_language: lang, issue_settings: { ai_worktree_name: true, closes_instruction: true } },
      },
    } as never);
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
  }, [state, theme, lang]);
  return (
    <TooltipProvider>
      <InterfaceLanguageBoundary />
      <div className="w-[640px] bg-background p-lg" data-gallery-scene="hide-ai-settings" data-scene-ready="true">
        <HideAiTab actions={actions} />
      </div>
    </TooltipProvider>
  );
}
