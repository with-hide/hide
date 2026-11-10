// The rules behind Settings › Mission (PRD software-factory-conductor D-31,
// B48): the choices the page offers and the value each edit sends. The
// core checks the value before it keeps it (`ConductorSettings::validate`);
// these mirror its bounds so the page never offers what it would refuse.

import { agentAdapter } from "./agentAdapters";
import { PROVIDER_KINDS, type ProfileItem, type ProviderKind } from "./agentPicker";
import { agentItems, BUILT_IN_PROFILE, listedProfiles, type AgentItem } from "./agentProfiles";
import type { ConductorCandidate, ConductorSettings, Device, MissionStrength } from "./snapshot";

/** The cap's range (`CAP_LIMIT`) and the candidate list's (`CANDIDATE_LIMIT`). */
export const CAP_LIMIT = 16;
export const CANDIDATE_LIMIT = 16;
export const CAP_CHOICES: readonly number[] = Array.from({ length: CAP_LIMIT }, (_, index) => index + 1);

/** The verification strengths, lightest first (`mission_strength`). */
export const STRENGTHS: readonly MissionStrength[] = ["fast", "normal", "thorough"];

/** What the core keeps before the operator changed anything. */
export const DEFAULT_CONDUCTOR: ConductorSettings = { concurrency: 4, default_scope: "pr", candidates: [], default_strength: "normal", clean_merged: true };

/** The start kind a candidate names, as the start panel lists it; null for an agent Hide cannot start. */
export function candidateAgent(agent: string): ProviderKind | null {
  const row = agentAdapter(agent);
  return row?.can_start ? (row.agent_cli as ProviderKind) : null;
}

/** The efforts the agent takes; none for one that takes no effort, which is then sent none (B9). */
export function effortsOf(agent: string): readonly string[] {
  return agentAdapter(agent)?.efforts ?? [];
}

/** The value with one more candidate: the first agent not listed yet, else the first agent. */
export function addCandidate(settings: ConductorSettings): ConductorSettings {
  if (settings.candidates.length >= CANDIDATE_LIMIT) return settings;
  const used = new Set(settings.candidates.map((candidate) => candidateAgent(candidate.agent)));
  const agent = PROVIDER_KINDS.find((kind) => !used.has(kind)) ?? PROVIDER_KINDS[0];
  if (!agent) throw new Error("No agent kind Hide can start");
  return { ...settings, candidates: [...settings.candidates, { agent, purpose: "" }] };
}

/** The value with candidate `index` replaced, or taken out for null. */
export function replaceCandidate(settings: ConductorSettings, index: number, next: ConductorCandidate | null): ConductorSettings {
  const candidates = next === null ? settings.candidates.filter((_, at) => at !== index) : settings.candidates.map((candidate, at) => (at === index ? next : candidate));
  return { ...settings, candidates };
}

/** The profile a candidate names, by its id on this machine; `default` for the agent's built-in one, which a row kept before profiles names by leaving it out (PRD agent-profiles-phase2 B14). */
export function candidateProfile(candidate: ConductorCandidate): string {
  return candidate.profile ?? BUILT_IN_PROFILE;
}

/** The row an item picked in its agent cell makes: that agent and profile, with the model and effort cleared since they were the other item's, and no profile sent for the built-in one, so it is kept as a row was before profiles. */
export function pickItem(candidate: ConductorCandidate, item: Pick<AgentItem, "kind" | "profile">): ConductorCandidate {
  return { agent: item.kind, ...(item.profile === BUILT_IN_PROFILE ? {} : { profile: item.profile }), purpose: candidate.purpose };
}

/**
 * The agent cell's list on this machine (`device`, the one the candidates'
 * profiles are of): every agent Hide starts, each with its built-in item
 * first and then the profiles the machine lists (B7). `named` says some
 * agent has a profile of its own, when the items read by the agents'
 * product names as the start panel's do; with none the cell names the
 * agents as it always did (B1).
 */
export function candidateItems(device: Device | undefined): { items: AgentItem[]; named: boolean } {
  const items = PROVIDER_KINDS.flatMap((kind) => agentItems(device, kind));
  return { items, named: items.some((item) => item.name !== null) };
}

/** A candidate's profile as `catalogFor` asks for it, its id and its version now on `device`; null for the built-in item and for a profile the machine no longer lists. */
export function candidateProfileItem(device: Device | undefined, profile: string): ProfileItem | null {
  if (profile === BUILT_IN_PROFILE) return null;
  const row = listedProfiles(device)?.profiles.find((listed) => listed.id === profile);
  return row ? { id: row.id, version: row.version } : null;
}

/** Whether the candidate's profile is one this machine no longer lists (B15): such a row admits no worker, and its item cannot be picked again. */
export function candidateGone(candidate: ConductorCandidate, items: readonly AgentItem[]): boolean {
  const kind = candidateAgent(candidate.agent);
  const profile = candidateProfile(candidate);
  return profile !== BUILT_IN_PROFILE && !items.some((item) => item.kind === kind && item.profile === profile);
}
