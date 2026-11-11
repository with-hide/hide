// Where a mission node's result goes after the work, as stations on its card
// (PRD mission-destination D-13-D-16, D-37, B24-B28): the one computation the
// desktop's map card, its hover and the phone's card read, from fields both
// carry, so the two screens draw one line.
//
// A node that changes code (`change`, `external`) has a line. A mission's own
// change starts at `작업`, then for a pull request `PR` (`#n` once it exists,
// the newest with `+N`) and `승인`, then `볼 것` when its plan has the
// operator look, and last the branch its result lands in. An outside node has
// no work in the mission, so its line starts at what it watches: `↗#n → 승인`
// for a pull request, `↗ <branch>` for a branch, then where that goes. A node
// that changes no code (`research`, `verify`) has none. Every station's state
// comes from the core's values; the screen decides nothing the core did not
// say.

import type { TFunction } from "i18next";
import type { MissionDelivery, MissionExternal, MissionNode, MissionNodeBrief, MissionOutside } from "./snapshot";

/** What the station line reads of a node: the shared rules' fields, and the pull request's base and count the line names. */
export type StationNode = Pick<MissionNodeBrief, "kind" | "status" | "pr" | "pr_state" | "landing" | "gate" | "merge"> & {
  delivery: MissionDelivery | null;
  base_pr: number | null;
  external?: MissionExternal | null;
  /** The branch the current pull request merges into, as GitHub last answered; absent until it answered. */
  pr_base?: string | null;
  /** How many pull requests the node holds besides its current one (`+N`, D-37). */
  pr_more?: number;
  /** Where an outside node's work goes and how its pull request stands (D-14); absent from a core before it. */
  outside?: MissionOutside | null;
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
  | { kind: "branch"; state: StationState; branch: string }
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
 * known is left out rather than named (B17). A closed mission's work
 * stopped, so no station of it is lit as current, as no edge of its map is.
 */
export function nodeStations(node: StationNode, destination: string | null, open: boolean): Station[] | null {
  if (!changesCode(node)) return null;
  const stations = node.kind === "external" ? outsideStations(node, destination) : ownStations(node, destination);
  if (!stations) return null;
  return open ? stations : stations.map((station) => (station.state === "current" ? { ...station, state: "todo" } : station));
}

/** A mission's own change: its work, its pull request when it has one, the operator's look, and the branch it lands in. */
function ownStations(node: StationNode, destination: string | null): Station[] | null {
  const pr = node.pr;
  const byPr = node.delivery !== "direct" || pr != null;
  const done = node.status === "done";
  const pull = node.pr_state;
  // The work goes on while there is no pull request, or while its checks failed, it is a draft or it was closed unmerged.
  const fixing = pull === "failed" || pull === "draft" || pull === "closed";
  const worked = done || (byPr && pr != null && !fixing);
  const stations: Station[] = [{ kind: "work", state: worked ? "done" : node.status === "working" ? "current" : "todo" }];
  let reached = worked;
  if (byPr) {
    const approved = pull === "mergeable" || pull === "merged" || done;
    const prState: StationState = pull === "failed" ? "failed" : pull === "closed" ? "closed" : approved ? "done" : pr != null && worked ? "current" : "todo";
    stations.push({ kind: "pr", state: prState, number: pr, more: node.pr_more ?? 0, outside: false }, { kind: "approve", state: approved ? "done" : "todo" });
    reached = approved;
  }
  if (node.gate) {
    const passed = node.gate.state === "passed";
    stations.push({ kind: "gate", state: passed ? "done" : node.gate.state === "open" ? "due" : "ahead" });
    reached = reached && passed;
  }
  const place = node.base_pr != null ? { kind: "stack" as const, pr: node.base_pr } : branchPlace(node.pr_base ?? destination, destination);
  if (place) {
    const person = node.merge?.by === "operator";
    // In only once all of it is: a node merged by pull request by its merge's own state, which an older pull request still open holds back however its newest stands; any other by the landing the core judged.
    const landed = node.merge ? node.merge.state === "merged" : node.landing?.state === "landed";
    const state: StationState = landed ? "landed" : person ? (node.merge?.state === "due" ? "due" : "ahead") : reached ? "current" : "todo";
    stations.push({ kind: "land", state, place, person });
  }
  // A line of `작업` alone says nothing the card's mark does not (D-17 dropped a one-station line): a direct change whose destination the core cannot read draws none.
  return stations.length < 2 ? null : stations;
}

/**
 * An outside node (D-07, D-14): no `작업`, since none of its work is the
 * mission's. A pull request is `↗#n → 승인 → <its base>` lit by the state
 * GitHub last answered; one closed unmerged lights nothing after it, and one
 * not read yet lights nothing. A branch is `↗ <branch> → <the default
 * branch it is judged against>`. Its node done is the result in, whatever
 * else was read.
 */
function outsideStations(node: StationNode, destination: string | null): Station[] | null {
  const external = node.external;
  if (!external) return null;
  const done = node.status === "done";
  const place = branchPlace(node.outside?.base ?? null, destination);
  if ("branch" in external) {
    const stations: Station[] = [{ kind: "branch", state: done ? "done" : node.status === "working" ? "current" : "todo", branch: external.branch }];
    if (place) stations.push({ kind: "land", state: done ? "landed" : "todo", place, person: false });
    return stations;
  }
  const pull = done ? "merged" : (node.outside?.state ?? null);
  const approved = pull === "mergeable" || pull === "merged";
  const prState: StationState = pull === "failed" ? "failed" : pull === "closed" ? "closed" : approved ? "done" : pull === "pending" || pull === "draft" ? "current" : "todo";
  const stations: Station[] = [{ kind: "pr", state: prState, number: external.pr, more: 0, outside: true }, { kind: "approve", state: approved ? "done" : "todo" }];
  if (place) stations.push({ kind: "land", state: pull === "merged" ? "landed" : pull === "mergeable" ? "current" : "todo", place, person: false });
  return stations;
}

/** The last station's branch: the destination drawn plainly, another branch bold; bold needs a destination to differ from, so with none read the branch is drawn plainly. */
function branchPlace(branch: string | null, destination: string | null): LandPlace | null {
  if (!branch) return null;
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
    case "branch":
      return `↗ ${station.branch}`;
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
