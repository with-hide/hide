import { useMemo } from "react";
import { useAgentLogosShown } from "../agentLogoMode";
import type { Actions } from "../actions";
import { deviceScope } from "../agentScope";
import { frontDeviceId } from "../devices";
import { machineName } from "../screenMachine";
import { useInterfaceTranslation } from "../i18n/client";
import { MissionTakeOver } from "../MissionTakeOver";
import { cn } from "../lib/utils";
import { missionAskWhat, missionAsks, projectAsks, safetyNetLine, type MissionAsk } from "../missions";
import { catalogWorkspaces, type SnapshotRest } from "../snapshot";
import { useShellStore } from "../store";
import { AskLine } from "./agent-tree";
import { DeviceChip } from "./device-chip";
import { Elapsed } from "./elapsed";
import { KindIcon } from "./kind-mark";
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

/**
 * The missions every device's scope names as its own Needs You items, the
 * front device's first (operator decision 2026-11-08): the sidebar's Needs
 * You lists every device's, each mission row naming a device not in front.
 */
export function useEveryDeviceMissionAsks(frontId: string): MissionAsk[] {
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const missions = useShellStore((s) => (s.rest?.missions?.failed ? null : s.rest?.missions?.items) ?? null);
  return useMemo(() => {
    if (!missions || !devices) return [];
    const ordered = [...devices.filter((device) => device.id === frontId), ...devices.filter((device) => device.id !== frontId)];
    return ordered.flatMap((device) => missionAsks(missions, device.agent_scope.mission_asks ?? []));
  }, [missions, devices, frontId]);
}

/** The name of the device a mission works on while another device is in front, for its row's chip. */
function useMissionDevice(ask: MissionAsk): string | null {
  const { t } = useInterfaceTranslation();
  const deviceId = ask.mission.device_id;
  return useShellStore((s) => (deviceId === frontDeviceId(s.rest) ? null : machineName(s.rest?.navigator?.devices, deviceId, t)));
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

/**
 * The line a carried ask reads: for the mission's oldest waiting node
 * `확인 필요 · <node> · <why>`, `외 N건`, a reached gate naming what to look at
 * after the node (PRD mission-human-gate D-23); for its safety net the held
 * count and what is missing (`걸린 일 N건 · Conductor가 아직 풀지 못했어요`).
 */
export function MissionAskLine({ ask, className }: { ask: MissionAsk; className?: string }) {
  const { t } = useInterfaceTranslation();
  if (ask.kind === "net") return <AskLine ask={{ verb: "answer", what: safetyNetLine(ask.net, t), more: ask.more }} className={className} />;
  const look = ask.attention.reason === "human_gate" ? ask.node.gate?.look : undefined;
  const node = look ? `${ask.node.title} · ${look}` : ask.node.title;
  const line = { verb: "answer" as const, what: null, more: ask.more, subject: { cause: "mission_attention" as const, node, reason: ask.attention.reason, unmet: ask.attention.unmet_nodes } };
  return <AskLine ask={line} className={className} />;
}

/**
 * 확인하기, which opens the mission where its 내 차례 card says what waits
 * and offers its choices, wherever the mission is its own Needs You item;
 * led by 지휘 다시 열기 while no Conductor holds the mission, and for a safety
 * net by 터미널로 이동 to the Conductor that has not settled it. The choices
 * are not drawn here: read without the card's context they left the operator
 * guessing (operator report on M41).
 */
export function MissionAskChoices({ ask, actions, className }: { ask: MissionAsk; actions: Actions; className?: string }) {
  const { t } = useInterfaceTranslation();
  const present = ask.mission.conductor.present;
  return (
    <span className={cn("flex min-w-0 flex-wrap items-center gap-xs", className)} data-mission-ask-choices={ask.mission.id}>
      {present ? null : <MissionTakeOver mission={ask.mission} actions={actions} />}
      {present && ask.kind === "net" ? (
        <Button size="sm" variant="secondary" onClick={() => actions.openAgent(ask.mission.conductor.pane_id)} data-mission-ask-conductor={ask.mission.id}>
          {t("mission.toTerminal")}
        </Button>
      ) : null}
      <Button size="sm" variant="secondary" onClick={() => actions.openMission(ask.mission.id)} data-mission-ask-look={ask.mission.id}>
        {t("missionScreen.pill.attention")}
      </Button>
    </span>
  );
}

/**
 * A mission that is its own Needs You item (PRD mission-node-simple D-19,
 * B14): no Conductor or worker row stands to carry what waits on the
 * operator, so the mission's title is the row, with the Needs You mark and
 * the line a carried one reads (`확인 필요 · <node> · <why>`, `외 N건`).
 * A safety net reads its held count instead (`MissionAskLine`). Under it
 * are 지휘 다시 열기 while no Conductor holds the mission, which starts one,
 * 터미널로 이동 for a safety net its Conductor holds, and 확인하기; opening
 * the row, or 확인하기, opens the mission's map.
 * It reads as the agent rows beside it do: the mission's mark in the logo's
 * place (shown as the logos are), how long the node has waited at the end of
 * the title line, the project as the place line, and the buttons on the
 * columns of the lines above.
 * The 프로젝트 list's Needs You and the 태스크 list's 내 차례 both draw it;
 * A mission of another device than the one in front names it with the
 * device chip, as an agent row of that device does.
 * `place` is false where the group already names the project, and `data-*`
 * lands on the row's open button, such as the 태스크 list's arrow-key mark.
 */
export function SidebarMissionRow({ ask, inset, actions, place: placed = true, ...data }: { ask: MissionAsk; inset: string; actions: Actions; place?: boolean } & Record<`data-${string}`, string>) {
  const { t } = useInterfaceTranslation();
  const logos = useAgentLogosShown();
  const project = useMissionPlace(ask);
  const place = placed ? project : null;
  const device = useMissionDevice(ask);
  const name = [ask.mission.title, device, t("mission.attention"), missionAskWhat(ask, t), place].filter(Boolean).join(", ");
  return (
    <li data-mission-ask={ask.mission.id} data-mission-ask-node={ask.kind === "node" ? ask.node.key : undefined} data-mission-ask-net={ask.kind === "net" || undefined} className={cn("group/row relative flex items-start gap-xs rounded-sm py-xs pr-xs text-body hover:bg-accent")} style={{ paddingLeft: inset }}>
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
          {device ? <DeviceChip label={device} className="pointer-events-none max-w-2/5" /> : null}
          <Elapsed since={ask.since_unix_ms} aria-hidden="true" className="pointer-events-none shrink-0 font-mono text-caption text-muted-foreground" data-mission-ask-elapsed="true" />
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
