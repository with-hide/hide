import { itemName, missingItemName } from "../agentProfiles";
import { Status, type Tone } from "../components/settings-rows";
import { percentOf, providerReason, resetTime, type AiItem } from "../hideAi";
import { useInterfaceTranslation } from "../i18n/client";
import { requireInterfaceLanguage } from "../i18n/locale";
import { coreMachineAway, machineName } from "../screenMachine";
import type { AiProvider, AiRefusal } from "../snapshot";
import { useShellStore } from "../store";

/** How the Hide AI tab words an agent's state and a refusal; one place so the Runs on row, the fallback rows and the Add menu agree. */
export function useHideAiWords() {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  // Hide AI runs the core machine's agents: seen from another machine, a sign-in that lapsed is named there (B15).
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const coreMachine = coreMachineAway(devices, t);
  const time = (unixMs: number) => resetTime(language, unixMs);
  const onMachine = (provider: AiProvider, named: string): string =>
    provider.device_id ? `${named} · ${machineName(devices, provider.device_id, t)}` : named;
  /**
   * An item as the tab names it, the way the start panel does (PRD agent-profiles-phase2 D-01): "Claude Code",
   * "Claude Code · work", a profile its machine no longer lists "Claude Code · Missing profile" (B21), and on a
   * connected device with the device after it, "Claude Code · WSL Ubuntu" (PRD wsl-device B11).
   */
  const name = (provider: AiProvider | AiItem): string => {
    const item = "profile" in provider ? provider : null;
    if (item?.missing) return onMachine(provider, missingItemName(t, provider.label));
    return onMachine(provider, itemName(t, provider.label, item?.profileName ?? item?.profile ?? null));
  };
  /** One short state: "Signed in", "Out of usage until 3:10 PM". */
  const state = (provider: AiProvider): { tone: Tone; text: string } => {
    switch (providerReason(provider)) {
      case "ready":
        // An agent with no sign-in check is ready only because its program was found, so it does not claim to be signed in.
        return provider.login_checked === false ? { tone: "muted", text: t("hideAi.state.loginUnchecked") } : { tone: "ok", text: t("hideAi.state.ready") };
      case "needs_login":
        // A device's row is named with its machine already; the core machine's is named only when this window is elsewhere.
        return { tone: "warn", text: coreMachine && !provider.device_id ? t("hideAi.state.needsLoginOn", { machine: coreMachine }) : t("hideAi.state.needsLogin") };
      case "usage_limited":
        return {
          tone: "warn",
          text: provider.retry_at_ms ? t("hideAi.state.usageLimitedUntil", { time: time(provider.retry_at_ms) }) : t("hideAi.state.usageLimited"),
        };
      case "not_installed":
        return { tone: "warn", text: t("hideAi.state.notInstalled") };
      case "read_only":
        return { tone: "warn", text: t("hideAi.state.readOnly") };
      case "unsupported":
        return { tone: "warn", text: t("hideAi.state.unsupported") };
      case "unread":
        return { tone: "pending", text: t("hideAi.state.unread") };
      default:
        return { tone: "warn", text: t("hideAi.state.unavailable") };
    }
  };
  /**
   * An agent's state with its week: "Signed in · 86% this week", or past the
   * limit "Past your limit · 86% this week · resets Mon 9:00". An agent with
   * no figure, or one that is not signed in, shows the state alone (D-05, D-07).
   */
  const stateWithUsage = (provider: AiProvider): { tone: Tone; text: string } => {
    const base = state(provider);
    const weekly = provider.weekly;
    if (!weekly || providerReason(provider) !== "ready" || provider.login_checked === false) return base;
    const percent = percentOf(weekly.used_percent);
    if (provider.past_limit) {
      return {
        tone: "warn",
        text: weekly.resets_at_ms
          ? t("hideAi.state.pastLimitUntil", { percent, time: time(weekly.resets_at_ms) })
          : t("hideAi.state.pastLimit", { percent }),
      };
    }
    return { tone: base.tone, text: t("hideAi.state.weekly", { state: base.text, percent }) };
  };
  /** Why Runs on is not answering, as a sentence about the item: "Claude Code · work is out of usage until 3:10 PM". */
  const refusal = (provider: AiProvider | AiItem, why: AiRefusal): string => {
    const agent = name(provider);
    switch (why.reason) {
      case "usage_limited":
        return why.retry_at_ms
          ? t("hideAi.reason.usageLimitedUntil", { agent, time: time(why.retry_at_ms) })
          : t("hideAi.reason.usageLimited", { agent });
      case "past_limit":
        return why.retry_at_ms
          ? t("hideAi.reason.pastLimitUntil", { agent, limit: why.limit ?? "", time: time(why.retry_at_ms) })
          : t("hideAi.reason.pastLimit", { agent, limit: why.limit ?? "" });
      case "needs_login":
        return coreMachine && !provider.device_id ? t("hideAi.reason.needsLoginOn", { agent, machine: coreMachine }) : t("hideAi.reason.needsLogin", { agent });
      case "not_installed":
        return t("hideAi.reason.notInstalled", { agent });
      case "unsupported":
        return t("hideAi.reason.unsupported", { agent });
      // The item's name is the reason: its profile is gone (B21).
      case "profile_missing":
        return onMachine(provider, missingItemName(t, provider.label));
      default:
        return t("hideAi.reason.unavailable", { agent });
    }
  };
  /** The line that asks for a sign-in when no agent is signed in, on the machine whose agents Hide AI runs. */
  const signIn = (agent: string): string => (coreMachine ? t("hideAi.signInOn", { agent, machine: coreMachine }) : t("hideAi.signIn", { agent }));
  return { name, state, stateWithUsage, refusal, signIn };
}

/** An agent's state as symbol plus words, never colour alone (design 7). */
export function ProviderState({ provider }: { provider: AiProvider | AiItem }) {
  const { stateWithUsage } = useHideAiWords();
  const { tone, text } = stateWithUsage(provider);
  return (
    <Status tone={tone} data-ai-state={`${"profile" in provider && provider.profile ? `${provider.id}#${provider.profile}` : provider.id}:${provider.state}`}>
      {text}
    </Status>
  );
}
