// The rules behind Settings › Conductor (PRD software-factory-conductor D-31,
// B48): the choices the page offers and the whole value each edit sends. The
// core checks the value before it keeps it (`ConductorSettings::validate`);
// these mirror its bounds so the page never offers what it would refuse.

import { agentAdapter } from "./agentAdapters";
import { PROVIDER_KINDS, type ProviderKind } from "./agentPicker";
import type { ConductorCandidate, ConductorSettings } from "./snapshot";

/** The cap's range (`CAP_LIMIT`) and the candidate list's (`CANDIDATE_LIMIT`). */
export const CAP_LIMIT = 16;
export const CANDIDATE_LIMIT = 16;
export const CAP_CHOICES: readonly number[] = Array.from({ length: CAP_LIMIT }, (_, index) => index + 1);

/** What the core keeps before the operator changed anything. */
export const DEFAULT_CONDUCTOR: ConductorSettings = { concurrency: 4, default_scope: "pr", candidates: [] };

/** The start kind a candidate names, as the start panel lists it; null for an agent Hide cannot start. */
export function candidateAgent(agent: string): ProviderKind | null {
  const row = agentAdapter(agent);
  return row?.can_start ? (row.herdr_kind as ProviderKind) : null;
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
