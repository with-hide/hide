import { useMemo } from "react";
import { useAgentLogosShown } from "../agentLogoMode";
import type { Actions } from "../actions";
import { deviceScope } from "../agentScope";
import { useInterfaceTranslation } from "../i18n/client";
import { MissionTakeOver } from "../MissionTakeOver";
import { cn } from "../lib/utils";
import { missionAsks, projectAsks, type MissionAsk } from "../missions";
import { catalogWorkspaces, type SnapshotRest } from "../snapshot";
import { useShellStore } from "../store";
import { AskLine } from "./agent-tree";
import { Elapsed } from "./elapsed";
import { KindIcon } from "./kind-mark";
import { AttentionChoices } from "./mission-attention";
import { Button } from "./ui/button";
import { Hint } from "./ui/tooltip";
import { FoldLane } from "./sidebar-agent-row";
import { SessionRowBody } from "./session-row";
import { StatusMark } from "./status-mark";

/**
 * The missions `deviceId`'s scope names as its own Needs You items, the ones
 * its Needs You number counts; with `projectId`, only that project's.
 */
export function useMissionAsks(deviceId: string | null | undefined, projectId: string | null = null): MissionAsk[] {
  const ids = useShellStore((s) => (deviceId ? deviceScope(s.rest, deviceId)?.mission_asks : undefined));
  const missions = useShellStore((s) => (s.rest?.missions?.failed ? null : s.rest?.missions?.items) ?? null);
  return useMemo(() => (missions && ids ? projectAsks(missionAsks(missions, ids), projectId) : []), [missions, ids, projectId]);
}

/** `useMissionAsks` read once from a snapshot, for a count drawn outside a render's subscription. */
export function deviceMissionAsks(rest: SnapshotRest | null, deviceId: string): MissionAsk[] {
  const missions = rest?.missions?.failed ? null : rest?.missions?.items;
  const ids = deviceScope(rest, deviceId)?.mission_asks;
  return missions && ids ? missionAsks(missions, ids) : [];
}

/** The project a mission works in, as a row's place line names it; none while the catalog does not hold it. */
export function useMissionPlace(ask: MissionAsk): string | null {
  const { workspace_id, device_id } = ask.mission;
  return useShellStore((s) => catalogWorkspaces(s.rest).find((row) => row.id === workspace_id && row.device_id === device_id)?.label ?? null);
}

/** The line a carried ask reads, for the mission's oldest waiting node: `확인 필요 · <node> · <why>`, `외 N건`. */
export function MissionAskLine({ ask, className }: { ask: MissionAsk; className?: string }) {
  const line = { verb: "answer" as const, what: null, more: ask.more, subject: { cause: "mission_attention" as const, node: ask.node.title, reason: ask.attention.reason } };
  return <AskLine ask={line} className={className} />;
}

/** 이어받기 and the node's choices for its cause, wherever the mission is its own Needs You item. */
export function MissionAskChoices({ ask, actions, className }: { ask: MissionAsk; actions: Actions; className?: string }) {
  return (
    <span className={cn("flex flex-wrap items-center gap-xs", className)} data-mission-ask-choices={ask.mission.id}>
      <MissionTakeOver mission={ask.mission} actions={actions} />
      <AttentionChoices key={`${ask.node.key}-${ask.attention.since_unix_ms}`} mission={ask.mission} node={ask.node} attention={ask.attention} actions={actions} />
    </span>
  );
}

/**
 * A mission that is its own Needs You item (PRD mission-node-simple D-19,
 * B14): no Conductor or worker row stands to carry what waits on the
 * operator, so the mission's title is the row, with the Needs You mark and
 * the line a carried one reads (`확인 필요 · <node> · <why>`, `외 N건`).
 * Under it are 이어받기, which starts a Conductor for the mission, and the
 * node's own choices for its cause; opening the row opens the mission's map.
 * It reads as the agent rows beside it do: the mission's mark in the logo's
 * place (shown as the logos are), how long the node has waited at the end of
 * the title line, the project as the place line, and the choices on the
 * columns of the lines above.
 * The 프로젝트 list's Needs You and the 태스크 list's 내 차례 both draw it;
 * `place` is false where the group already names the project, and `data-*`
 * lands on the row's open button, such as the 태스크 list's arrow-key mark.
 */
export function SidebarMissionRow({ ask, inset, actions, place: placed = true, ...data }: { ask: MissionAsk; inset: string; actions: Actions; place?: boolean } & Record<`data-${string}`, string>) {
  const { t } = useInterfaceTranslation();
  const logos = useAgentLogosShown();
  const project = useMissionPlace(ask);
  const place = placed ? project : null;
  const name = [ask.mission.title, t("mission.attention"), ask.node.title, place].filter(Boolean).join(", ");
  return (
    <li data-mission-ask={ask.mission.id} data-mission-ask-node={ask.node.key} className={cn("group/row relative flex items-start gap-xs rounded-sm py-xs pr-xs text-body hover:bg-accent")} style={{ paddingLeft: inset }}>
      <Hint label={name} reveals>
        <Button
          variant="text"
          size="inline"
          aria-label={name}
          data-mission-ask-open={ask.mission.id}
          {...data}
          className="absolute inset-0 rounded-sm focus-visible:ring-inset"
          onClick={() => actions.openMission(ask.mission.id)}
        />
      </Hint>
      <SessionRowBody
        markClassName="pointer-events-none"
        mark={<>
          <StatusMark mark="turn" />
          {logos ? <KindIcon kind="mission" className="size-(--size-agent-badge-compact)" /> : null}
        </>}
        head={<>
          <span aria-hidden="true" className="pointer-events-none min-w-0 flex-1 truncate text-foreground">{ask.mission.title}</span>
          <Elapsed since={ask.attention.since_unix_ms} aria-hidden="true" className="pointer-events-none shrink-0 font-mono text-caption text-muted-foreground" data-mission-ask-elapsed="true" />
          <FoldLane />
        </>}
      >
        <span aria-hidden="true" className="pointer-events-none flex min-w-0 leading-(--size-sidebar-line-detail)"><MissionAskLine ask={ask} className="flex-1" /></span>
        {place ? (
          <span aria-hidden="true" data-mission-ask-place={place} className="pointer-events-none truncate text-caption leading-(--size-sidebar-line-detail) text-muted-foreground">
            {place}
          </span>
        ) : null}
        <MissionAskChoices ask={ask} actions={actions} className="relative z-10 mt-xs" />
      </SessionRowBody>
    </li>
  );
}
