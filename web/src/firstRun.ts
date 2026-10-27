// The first-run wizard's rules (PRD first-run-onboarding); `FirstRunWizard.tsx`
// draws them. Which step is on screen is the core's (`ui_state.first_run`);
// everything here reads the snapshot and decides nothing the core owns.

import { AGENT_ADAPTERS } from "./agentAdapters";
import type { ProviderKind } from "./agentPicker";
import type { NotificationPermission } from "./host";
import { hookPart, type AgentPart } from "./settings/agentRows";
import { FIRST_RUN_VERSION, type AgentUpdate, type AiProvider, type FirstRunStep, type HerdrProfiles, type Kit, type KitAgent, type KitPiece, type KitWhy, type SnapshotRest } from "./snapshot";

/** The desktop route includes saved devices; a browser tab has its first step alone. */
export type WizardStep = Exclude<FirstRunStep, "done">;

/**
 * Failed or unread discovery has its own retry step; an observed empty list skips it. Moving the operator's
 * older Herdr comes first, and only while there is one to move (PRD herdr-takeover B1, B17).
 */
export function wizardSteps(profiles: HerdrProfiles | undefined, herdr = false): readonly WizardStep[] {
  const rest: readonly WizardStep[] = profiles?.state === "ready" && profiles.profiles.length === 0
    ? ["agents", "notifications", "project"]
    : ["agents", "devices", "notifications", "project"];
  return herdr ? ["herdr", ...rest] : rest;
}

/**
 * The step the first-run wizard shows, or null when it does not show: a Mac
 * that was never asked, a wizard that ended, progress saved by another flow
 * version, or, in a browser tab, any step past the first, which only the
 * desktop app has.
 */
export function firstRunStep(rest: SnapshotRest | null, desktop: boolean): WizardStep | null {
  const run = rest?.ui_state?.first_run;
  if (!run || run.version !== FIRST_RUN_VERSION || run.step === "done") return null;
  // A browser tab's wizard is the agent step alone; its Continue leaves a Herdr step unanswered (PRD herdr-takeover D-24).
  if (!desktop) return run.step === "agents" || run.step === "herdr" ? "agents" : null;
  return run.step;
}

/** The agents whose sign-in Hide checks (D-06); the others can only say they are installed. */
export const SIGN_IN_CHECKED: ReadonlySet<string> = new Set(["claude-code", "codex", "grok", "pi", "antigravity"]);

/** What a tile says under the agent's name (B8, B9, B49). */
export type TileStatus = "checking" | "ready" | "not_signed_in" | "installed" | "not_installed" | "unsupported" | "update_needed";

export function tileStatus(agent: KitAgent, providers: readonly AiProvider[] | undefined): TileStatus {
  if (agent.availability === "unsupported_system") return "unsupported";
  if (agent.availability !== "available") return "not_installed";
  // Its copy is older than Hide's minimum: the tile stays a switch, on as chosen, and says so; the update installs it.
  if (agent.update) return "update_needed";
  if (!SIGN_IN_CHECKED.has(agent.id)) return "installed";
  const provider = providers?.find((row) => row.agent === agent.id);
  if (!provider || provider.state === "unread") return "checking";
  if (provider.state === "needs_login") return "not_signed_in";
  // A usage limit is an account that is signed in.
  if (provider.state === "ready" || provider.state === "usage_limited") return "ready";
  return "installed";
}

/** The agents a tile marks as needing an update, in the tiles' order, each with what updates it. */
export function agentsToUpdate(agents: readonly KitAgent[]): (KitAgent & { update: AgentUpdate })[] {
  return agents.filter((agent): agent is KitAgent & { update: AgentUpdate } => agent.availability === "available" && agent.update != null);
}

/** The kit listed its agents and none is installed here: the step says so and links the two it names (B9). An empty list is a kit that has not listed them. */
export function noAgentInstalled(agents: readonly KitAgent[]): boolean {
  return agents.length > 0 && agents.every((agent) => agent.availability !== "available");
}

export const SUGGESTED_INSTALLS = ["claude-code", "codex"] as const;

/** One place under What changes: the path Hide writes, the agents it writes it for and which of their pieces it is (B11). */
export type ChangeRow = { path: string; labels: string[]; part: AgentPart };

export function accountHome(homeFolder: string | null | undefined): string | null {
  if (!homeFolder) return null;
  const cut = Math.max(homeFolder.lastIndexOf("/"), homeFolder.lastIndexOf("\\"));
  return cut > 0 ? homeFolder.slice(0, cut) : null;
}

/** `path` with the account `home` written as `~`, on every system (D-26); a path outside it as it is. */
export function underHome(path: string, home: string | null): string {
  if (!home) return path;
  if (path === home) return "~";
  const rest = path.slice(home.length);
  return path.startsWith(home) && (rest.startsWith("/") || rest.startsWith("\\")) ? `~${rest}` : path;
}

/**
 * What Continue changes on this Mac: for each agent left on, the files and
 * folders its pieces are written to (the hook file or its plugin or
 * extension, the skill, Herdr's integration), one row per place, so a skill
 * folder agents share is one row naming them all; then the `hide` command's
 * link. Each place is where the kit reports it, with the account home as `~`
 * (B11, D-26). A piece the agent does not have here (`absent`) writes nothing.
 */
export function whatChanges(agents: readonly KitAgent[], on: ReadonlySet<string>, kit: Kit | null | undefined, home: string | null = null): { rows: ChangeRow[]; cli: string | null } {
  const rows: ChangeRow[] = [];
  for (const agent of agents) {
    if (!on.has(agent.id)) continue;
    const pieces: [KitPiece | null | undefined, AgentPart][] = [[agent.hook, hookPart(agent)], [agent.skill, "skill"], [agent.herdr, "herdr"]];
    for (const [piece, part] of pieces) {
      if (!piece || piece.state === "absent" || !piece.location) continue;
      const path = underHome(piece.location, home);
      const shared = rows.find((row) => row.path === path);
      if (shared) shared.labels.push(agent.label);
      else rows.push({ path, labels: [agent.label], part });
    }
  }
  const cli = kit?.components.find((component) => component.id === "cli")?.location ?? null;
  return { rows, cli: cli === null ? null : underHome(cli, home) };
}

/** A chosen agent whose piece failed to go in, with the first reason the kit gave (B12). */
export type FailedAgent = { id: string; label: string; why: KitWhy | null };

/** The chosen agents that are on and have a failed piece: the step stays and names them (the core keeps it on the same rule). */
export function failedAgents(agents: readonly KitAgent[], chosen: readonly string[]): FailedAgent[] {
  return agents.flatMap((agent) => {
    if (!agent.enabled || !chosen.includes(agent.id)) return [];
    // A piece refused because the copy is too old is not a failure Try again fixes; the update installs it.
    const failed = [agent.hook, agent.skill, agent.herdr ?? null].find((piece) => piece?.state === "failed" && piece.why?.cause !== "update_agent");
    return failed ? [{ id: agent.id, label: agent.label, why: failed.why ?? null }] : [];
  });
}

function startKind(id: string): ProviderKind | null {
  const row = AGENT_ADAPTERS.find((adapter) => adapter.id === id && adapter.can_start);
  return row ? (row.herdr_kind as ProviderKind) : null;
}

/**
 * The agent the start panel opens on after the wizard's project (D-15): the
 * first agent that is on and signed in, in the order D-06 checks sign-in,
 * then the first agent that is on. With none on it is null, and no panel
 * opens: the empty Home's Start an agent is the way on (B28).
 */
export function firstStartAgent(agents: readonly KitAgent[], providers: readonly AiProvider[] | undefined): ProviderKind | null {
  const on = agents.filter((agent) => agent.enabled && agent.availability === "available" && startKind(agent.id) !== null);
  const signedIn = on.find((agent) => SIGN_IN_CHECKED.has(agent.id) && tileStatus(agent, providers) === "ready");
  const chosen = signedIn ?? on[0];
  return chosen ? startKind(chosen.id) : null;
}

/**
 * Whether the agent `kind` the start panel would run is one whose sign-in
 * Hide checks and that last read as signed out (B23); an agent Hide cannot
 * check, Terminal, or a sign-in not read yet says nothing (D-06).
 */
export function notSignedIn(kind: ProviderKind | "terminal", providers: readonly AiProvider[] | undefined): boolean {
  const id = AGENT_ADAPTERS.find((adapter) => adapter.can_start && adapter.herdr_kind === kind)?.id;
  if (!id || !SIGN_IN_CHECKED.has(id)) return false;
  return providers?.find((row) => row.agent === id)?.state === "needs_login";
}

/**
 * What the notification step and Settings > Notifications show (B14-B19, B30):
 * `off` before the operator turned it on, `reading` until the first answer
 * about the permission, `asking` while macOS's dialog waits for an answer,
 * `on`, `blocked` when macOS refuses, `unreadable` when Hide could not read
 * macOS's setting, `on_unchecked` where the system has no permission to read
 * (Windows, Linux), and `no_service` where nothing can be shown (a Linux
 * desktop with no notification service). On a Mac that already allows Hide
 * the wizard's step opens on `on` with the switch still off, and its Continue
 * turns the switch on (B16).
 */
export type NotificationState = "off" | "reading" | "asking" | "on" | "blocked" | "unreadable" | "on_unchecked" | "no_service";

export function notificationState(enabled: boolean, permission: NotificationPermission | null, place: "wizard" | "settings"): NotificationState {
  // A desktop with no notification service shows nothing whatever the switch says (D-24, B47, B48).
  if (permission === "no_service") return "no_service";
  if (permission === "unsupported") return enabled ? "on_unchecked" : "off";
  // macOS's refusal shows even with the switch off: turning it on would do nothing.
  if (permission === "denied") return "blocked";
  if (permission === "granted" && (enabled || place === "wizard")) return "on";
  if (!enabled) return "off";
  if (permission === null) return "reading";
  return permission === "not_determined" ? "asking" : "unreadable";
}

/** How often a screen showing the permission reads it again, so a change in System Settings shows within seconds (B16). */
export const PERMISSION_POLL_MS = 2000;

/** The id a notification about an agent carries, so its click and its withdrawal find it again. */
export const AGENT_NOTICE_PREFIX = "agent:";
