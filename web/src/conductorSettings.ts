// The rules behind Settings › Conductor (PRD software-factory-conductor D-31,
// B48): the choices the page offers and the whole value each edit sends. The
// core checks the value before it keeps it (`ConductorSettings::validate`);
// these mirror its bounds so the page never offers what it would refuse.

import { agentAdapter } from "./agentAdapters";
import { PROVIDER_KINDS, type ProviderKind } from "./agentPicker";
import type { ConductorCandidate, ConductorCheckDefault, ConductorSettings } from "./snapshot";

/** The cap's range (`CAP_LIMIT`) and the candidate list's (`CANDIDATE_LIMIT`). */
export const CAP_LIMIT = 16;
export const CANDIDATE_LIMIT = 16;
export const CAP_CHOICES: readonly number[] = Array.from({ length: CAP_LIMIT }, (_, index) => index + 1);

/** How many agents a default of several names (`CHECK_COUNT_LIMIT`); the core refuses fewer than two even while unused. */
export const CHECK_COUNT_CHOICES: readonly number[] = [2, 3, 4, 5];

/** How a node is checked before the operator changed anything (`CheckDefault::default`). */
export const DEFAULT_CHECK: ConductorCheckDefault = { mode: "conductor", rule: "all", count: 2, agent: null, model: null, effort: null, other_vendor: true };

export type CheckKind = "review" | "verify";
export type CheckerDefaults = Record<CheckKind, ConductorCheckDefault>;

export const DEFAULT_CHECKERS: CheckerDefaults = { review: DEFAULT_CHECK, verify: DEFAULT_CHECK };

/** What the core keeps before the operator changed anything. */
export const DEFAULT_CONDUCTOR: ConductorSettings = { concurrency: 4, default_scope: "pr", candidates: [], checkers: DEFAULT_CHECKERS };

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

/**
 * The check defaults with one of them replaced, sent whole because the core
 * replaces the value (PRD mission-map-git-criteria B36). Another agent takes
 * the CLI's default model and a Conductor-chosen effort, since the old ones
 * may be refused by the new agent.
 */
export function replaceCheck(checkers: CheckerDefaults, kind: CheckKind, next: ConductorCheckDefault): CheckerDefaults {
  const before = checkers[kind];
  const fresh = next.agent !== before.agent ? { ...next, model: null, effort: null } : next;
  return { ...checkers, [kind]: fresh };
}

/** Whether two check defaults say the same, an absent model or effort reading as none, as the core keeps them. */
export function sameCheckers(a: CheckerDefaults, b: CheckerDefaults): boolean {
  const same = (x: ConductorCheckDefault, y: ConductorCheckDefault) =>
    x.mode === y.mode && x.rule === y.rule && x.count === y.count && (x.agent ?? null) === (y.agent ?? null) && (x.model ?? null) === (y.model ?? null) && (x.effort ?? null) === (y.effort ?? null) && x.other_vendor === y.other_vendor;
  return same(a.review, b.review) && same(a.verify, b.verify);
}
