// What a mission applies of Settings › Mission, as its map's head says it in
// one grey line (PRD mission-destination D-35, D-50, B53): who merges, the
// worktree cleanup, the check strength and how many run at once, each
// compared with Settings so a value this mission changed stands out. The
// line is read only: changing a value is asking the Conductor.

import type { TFunction } from "i18next";
import type { ConductorSettings, Mission, MissionEvent, MissionNodeBrief } from "./snapshot";

export type MissionOptionKey = "merge" | "worktree" | "strength" | "cap";

/** One option: its short words on the line, its whole value on hover, whether this mission differs from Settings, and why when the core keeps a reason. */
export type MissionOption = { key: MissionOptionKey; short: string; value: string; changed: boolean; why: string | null };

/**
 * The options in the order the line reads them, merge first, so a scope
 * `pr` mission opens with `PR까지 · 머지는 전부 내가` (D-35). A mission from
 * before verification strength names none rather than Settings' (B16). A
 * reason is shown only where the core keeps one: the plan's strength change
 * is an event's line; a scope or cap change has none on the wire, so none is
 * made up.
 */
export function missionOptions(mission: Pick<Mission, "scope" | "strength" | "cap">, settings: ConductorSettings, events: readonly MissionEvent[] | null, t: TFunction<"translation">): MissionOption[] {
  const options: MissionOption[] = [
    { key: "merge", short: t(`mission.options.scope.${mission.scope}`), value: t(`mission.options.value.${mission.scope}`), changed: mission.scope !== settings.default_scope, why: null },
    { key: "worktree", short: t(settings.clean_merged ? "mission.options.worktree.clean" : "mission.options.worktree.keep"), value: t(settings.clean_merged ? "mission.options.value.clean" : "mission.options.value.keep"), changed: false, why: null },
  ];
  if (mission.strength) {
    const changed = mission.strength !== settings.default_strength;
    const event = changed ? [...(events ?? [])].reverse().find((one) => one.kind === "strength" && one.strength === mission.strength) : undefined;
    options.push({ key: "strength", short: t(`mission.strength.${mission.strength}`), value: t(`mission.strength.${mission.strength}`), changed, why: event?.line ?? null });
  }
  options.push({ key: "cap", short: t("mission.options.cap", { count: mission.cap }), value: t("mission.options.value.cap", { count: mission.cap }), changed: mission.cap !== settings.concurrency, why: null });
  return options;
}

/** Whether any node's result lands in a branch of the mission's own, so the head names the destination (B48): a mission's own change; research, verification and outside nodes land in none. */
export function landsInDestination(nodes: readonly Pick<MissionNodeBrief, "kind">[]): boolean {
  return nodes.some((node) => node.kind === "change");
}
