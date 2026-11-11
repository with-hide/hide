// Where a mission node's result goes after the work, as stations on its card
// (PRD mission-destination D-13-D-16, D-37, B24-B28): the one computation the
// desktop's map card, its hover and the phone's card read, from fields both
// carry, so the two screens draw one line.
//
// A node that changes code (`change`, `external`) has a line: `작업` first,
// then for a pull request `PR` (`#n` once it exists, the newest with `+N`)
// and `승인`, then `볼 것` when its plan has the operator look, and last the
// branch its result lands in. A node that changes no code (`research`,
// `verify`) has none. Every station's state comes from the core's values; the
// screen decides nothing the core did not say.

import type { TFunction } from "i18next";
import type { MissionDelivery, MissionExternal, MissionNode, MissionNodeBrief } from "./snapshot";

/** What the station line reads of a node: the shared rules' fields, and the pull request's base and count the line names. */
export type StationNode = Pick<MissionNodeBrief, "kind" | "status" | "pr" | "pr_state" | "landing" | "gate" | "merge"> & {
  delivery: MissionDelivery | null;
  base_pr: number | null;
  external?: MissionExternal | null;
  /** The branch the current pull request merges into, as GitHub last answered; absent until it answered. */
  pr_base?: string | null;
  /** How many pull requests the node holds besides its current one (`+N`, D-37). */
  pr_more?: number;
};

/** The desktop's node as the station line reads it: the base and count from its pull request list, the current one last. */
export function stationNode(node: MissionNode): StationNode {
  const current = node.prs.at(-1);
  return { ...node, pr_base: current?.base ?? null, pr_more: Math.max(node.prs.length - 1, 0) };
}

/**
 * A station's state (D-16, D-21): `todo` not reached, `current` where the
 * result is now, `done` passed (grey check), `failed` its checks failed,
 * `closed` closed unmerged; a planned human step is `ahead` (faint orange)
 * until it is the operator's to do now, `due` (yellow); `landed` is the last
 * station once the result is in.
 */
export type StationState = "todo" | "current" | "done" | "failed" | "closed" | "ahead" | "due" | "landed";

/** Where the last station says the result lands: the mission's destination, another branch, or the pull request it is stacked on (D-14). */
export type LandPlace = { kind: "destination"; branch: string } | { kind: "branch"; branch: string } | { kind: "stack"; pr: number };

export type Station =
  | { kind: "work"; state: StationState }
  | { kind: "pr"; state: StationState; number: number | null; more: number; outside: boolean }
  | { kind: "approve"; state: StationState }
  | { kind: "gate"; state: StationState }
  | { kind: "land"; state: StationState; place: LandPlace; person: boolean };

/** Whether a node's result goes into a branch, so it carries a station line (D-17): a change or an outside node. */
export function changesCode(node: Pick<MissionNodeBrief, "kind">): boolean {
  return node.kind === "change" || node.kind === "external";
}

/**
 * A node's stations, or null for a node that changes no code (D-13, D-17)
 * or whose line would be `작업` alone.
 * `destination` is the mission's destination branch as the core sends it,
 * null when the core could not read it. A last station whose branch is not
 * known is left out rather than named (B17): an outside node's, whose base
 * the core does not send, and any while the destination is unknown. A
 * closed mission's work stopped, so no station of it is lit as current, as
 * no edge of its map is.
 */
export function nodeStations(node: StationNode, destination: string | null, open: boolean): Station[] | null {
  if (!changesCode(node)) return null;
  const outsidePr = node.external && "pr" in node.external ? node.external.pr : null;
  const pr = node.pr ?? outsidePr;
  const byPr = node.kind === "external" ? outsidePr != null : node.delivery !== "direct" || node.pr != null;
  const done = node.status === "done";
  // An outside pull request's state is not read; its node is done once it merged (D-07).
  const pull = node.pr_state ?? (outsidePr != null && done ? "merged" : null);
  // The work goes on while there is no pull request, or while its checks failed, it is a draft or it was closed unmerged.
  const fixing = pull === "failed" || pull === "draft" || pull === "closed";
  const worked = done || (byPr && pr != null && !fixing);
  const stations: Station[] = [{ kind: "work", state: worked ? "done" : node.status === "working" ? "current" : "todo" }];
  let reached = worked;
  if (byPr) {
    const approved = pull === "mergeable" || pull === "merged" || done;
    const prState: StationState = pull === "failed" ? "failed" : pull === "closed" ? "closed" : approved ? "done" : pr != null && worked ? "current" : "todo";
    stations.push({ kind: "pr", state: prState, number: pr, more: node.pr_more ?? 0, outside: node.kind === "external" }, { kind: "approve", state: approved ? "done" : "todo" });
    reached = approved;
  }
  if (node.gate) {
    const passed = node.gate.state === "passed";
    stations.push({ kind: "gate", state: passed ? "done" : node.gate.state === "open" ? "due" : "ahead" });
    reached = reached && passed;
  }
  const place = landPlace(node, destination);
  if (place) {
    const person = node.merge?.by === "operator";
    const landed = node.landing?.state === "landed" || pull === "merged" || node.merge?.state === "merged";
    const state: StationState = landed ? "landed" : person ? (node.merge?.state === "due" ? "due" : "ahead") : reached ? "current" : "todo";
    stations.push({ kind: "land", state, place, person });
  }
  // A line of `작업` alone says nothing the card's mark does not (D-17 dropped a one-station line): a direct change whose destination the core cannot read draws none.
  if (stations.length < 2) return null;
  return open ? stations : stations.map((station) => (station.state === "current" ? { ...station, state: "todo" } : station));
}

function landPlace(node: StationNode, destination: string | null): LandPlace | null {
  if (node.base_pr != null) return { kind: "stack", pr: node.base_pr };
  // A pull request lands in the base GitHub answered for it, a mission's own change in the destination; an outside node's base is not sent.
  const branch = node.pr_base ?? (node.kind === "external" ? null : destination);
  if (!branch) return null;
  // Bold "another branch" needs a destination to differ from: with none read, GitHub's base is drawn plainly.
  return destination == null || branch === destination ? { kind: "destination", branch } : { kind: "branch", branch };
}

/**
 * Whether a node's own plan gives the operator its merge where the
 * mission's scope would not (D-10, D-45): the second-line `👤` strip's
 * condition. A scope `pr` mission leaves every merge to the operator and
 * draws no strip (D-35).
 */
export function mergeOverridden(scope: "pr" | "merge", node: Pick<MissionNodeBrief, "merge">): boolean {
  return scope === "merge" && node.merge?.by === "operator";
}

/** A station's fixed word (D-15): only the pull request's number is a number. */
export function stationLabel(station: Station, t: TFunction<"translation">): string {
  switch (station.kind) {
    case "work":
      return t("mission.station.work");
    case "pr":
      // An outside node's pull request carries `↗` in every state, its kind (B19).
      return station.number == null ? t("mission.station.pr") : `${station.outside ? "↗" : ""}#${station.number}${station.more > 0 ? ` +${station.more}` : ""}`;
    case "approve":
      return t("mission.station.approve");
    case "gate":
      return t("mission.gate.title");
    case "land":
      return station.place.kind === "stack" ? t("mission.station.onPr", { pr: String(station.place.pr) }) : station.place.branch;
  }
}

/**
 * The line as words, for the card's accessible name and its hover (B28): each
 * station's word and, past `todo`, its state, so the state is never only a
 * colour; the operator's merge is said on the last station.
 */
export function stationWords(stations: readonly Station[], t: TFunction<"translation">): string {
  return stations
    .map((station) => {
      const label = stationLabel(station, t);
      const person = station.kind === "land" && station.person ? ` · ${t("mission.merge.mine")}` : "";
      const state = station.state === "todo" ? "" : ` (${t(`mission.station.state.${station.state}`)})`;
      return `${label}${person}${state}`;
    })
    .join(" → ");
}
