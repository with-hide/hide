// The phone's gathered view (PRD device-connect-iroh D-56, D-69, B55, B62):
// 내 차례 and 읽을 것 of every computer this phone is paired with, in one
// list, each row marked with its computer. Each computer's core decides its
// own rows, order and folds; this joins them and says, per computer, why its
// rows are not there yet. Pure, so every rule is tested without a socket.

import { missionsOnWire, type AgentGroup, type GroupId, type GroupRow, type PhoneAutomation, type PhoneMission, type PhoneMissionOnWire } from "./protocol";
import type { Presence } from "./presence";

/** The groups the gathered view carries: what waits on the operator and what a notification announces. */
export const GATHERED_GROUPS: readonly GroupId[] = ["needs_you", "done"];

/** A saved computer the gathered view lists: its room and the name this phone keeps for it. */
export type GatheredHost = { room: string; name: string };

/** One computer's rows as they reach the view: the open computer's from its own list, another's from its watcher. */
export type GatherSource = { host: GatheredHost; here: boolean; groups: readonly AgentGroup[] };

/** A row of the gathered list, with the computer it came from; `here` rows belong to the computer this page is open on. */
export type GatheredRow = { host: GatheredHost; here: boolean; row: GroupRow };

/**
 * A gathered group: `count` is every computer's own count added (folded rows too), `rows` every computer's shown rows and `more`
 * every computer's folded rows, each newest first across computers, so no computer's own fold is decided twice.
 */
export type GatheredGroup = { group: GroupId; count: number; rows: GatheredRow[]; more: GatheredRow[] };

/** Why another computer's rows are not in the view: still connecting, not connected, or a phone protocol this bundle does not speak. */
export type HostLine = "connecting" | "offline" | "update";

/** When a row last moved, for the order across computers: an agent's state change, a sleeping conversation's sleep. */
function movedAt(row: GroupRow): number {
  return row.kind === "agent" ? (row.changed_at_unix_ms ?? 0) : row.since_unix_ms;
}

/** Newest first; rows that moved at the same instant keep their computer's order, the open computer's first. */
function newestFirst(rows: GatheredRow[]): GatheredRow[] {
  return rows.map((row, index) => ({ row, index })).sort((a, b) => movedAt(b.row.row) - movedAt(a.row.row) || a.index - b.index).map(({ row }) => row);
}

/** 내 차례 then 읽을 것, each joined across every source; a group no computer has is left out. */
export function gatherGroups(sources: readonly GatherSource[]): GatheredGroup[] {
  const gathered: GatheredGroup[] = [];
  for (const id of GATHERED_GROUPS) {
    let count = 0;
    const rows: GatheredRow[] = [];
    const more: GatheredRow[] = [];
    for (const source of sources) {
      const group = source.groups.find((candidate) => candidate.group === id);
      if (!group) continue;
      count += group.count;
      for (const row of group.rows) rows.push({ host: source.host, here: source.here, row });
      for (const row of group.more ?? []) more.push({ host: source.host, here: source.here, row });
    }
    if (count > 0) gathered.push({ group: id, count, rows: newestFirst(rows), more: newestFirst(more) });
  }
  return gathered;
}

/**
 * Another computer's state in the view: null once its list arrived (its rows say the rest), else the one line it shows. A computer
 * not heard from yet is connecting; one whose relay announced another phone protocol needs an update, whatever its socket does next.
 */
export function hostLine(presence: Presence | undefined): HostLine | null {
  if (!presence) return "connecting";
  if (presence.protocolMismatch !== undefined) return "update";
  if (!presence.online) return "offline";
  return presence.gathered ? null : "connecting";
}

/** An automation of another computer that stopped for a person: what its row in 내 차례 says, and which page it opens. */
export type GatheredAsk = { id: string; name: string; reason: NonNullable<PhoneAutomation["paused"]>["reason"] };

/** The automations a computer's `asks` names that are paused, as its own list draws them (`AutomationAskRows`); the rest is not kept. */
export function asksOf(automations: readonly PhoneAutomation[], asks: readonly string[]): GatheredAsk[] {
  return asks.flatMap((id) => {
    const automation = automations.find((candidate) => candidate.id === id);
    return automation?.paused ? [{ id, name: automation.name, reason: automation.paused.reason }] : [];
  });
}

/** A once automation of another computer whose run ended (mission-104 S-5): its name and the run its 내 차례 line words. */
export type GatheredLook = Pick<PhoneAutomation, "id" | "name" | "look_run" | "records">;

/** The automations a computer's `looks` names, each with only the run its line says; the rest is not kept. */
export function looksOf(automations: readonly PhoneAutomation[], looks: readonly string[]): GatheredLook[] {
  return looks.flatMap((id) => {
    const automation = automations.find((candidate) => candidate.id === id);
    if (!automation) return [];
    const records = automation.records.filter((record) => record.kind === "run" && record.number === automation.look_run);
    return [{ id, name: automation.name, look_run: automation.look_run, records }];
  });
}

/**
 * The open computer's own line in the gathered view: none once its list arrived (its rows say the rest, dimmed while it is out of
 * reach), else connecting or out of reach, so a page opened on a computer that is off still gathers the others (B62).
 */
export function hereLine(listed: boolean, unreachable: boolean): HostLine | null {
  if (listed) return null;
  return unreachable ? "offline" : "connecting";
}

/** Only a watcher's list of the groups the view carries: the rest of a computer's list is never kept. */
export function gatheredOf(groups: readonly AgentGroup[]): AgentGroup[] {
  return groups.filter((group) => GATHERED_GROUPS.includes(group.group));
}

/**
 * Whether the agents tab shows every computer (`all`) or the open one alone (`one`): one when the address says so, which a press on
 * a computer's chip writes, else all, so the gathered view is the list a phone paired with two computers or more returns to.
 */
export function listScopeOf(query: URLSearchParams): "all" | "one" {
  return query.has("one") ? "one" : "all";
}

/** The missions a computer's `asks` names, in that order, as its own list draws them (`MissionAskRows`); the rest is not kept. */
export function missionAsksOf(items: readonly PhoneMissionOnWire[], asks: readonly string[]): PhoneMission[] {
  return missionsOnWire(asks.flatMap((id) => items.filter((mission) => mission.id === id)));
}
