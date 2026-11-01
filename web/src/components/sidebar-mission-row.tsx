import type { Actions } from "../actions";
import { useInterfaceTranslation } from "../i18n/client";
import { MissionTakeOver } from "../MissionTakeOver";
import { cn } from "../lib/utils";
import type { MissionAsk } from "../missions";
import { AskLine } from "./agent-tree";
import { AttentionChoices } from "./mission-attention";
import { Button } from "./ui/button";
import { Hint } from "./ui/tooltip";
import { FoldLane } from "./sidebar-agent-row";
import { SessionRowBody } from "./session-row";
import { StatusMark } from "./status-mark";

/**
 * A mission that is its own Needs You item (PRD mission-node-simple D-19,
 * B14): no Conductor or worker row stands to carry what waits on the
 * operator, so the mission's title is the row, with the Needs You mark and
 * the line a carried one reads (`확인 필요 · <node> · <why>`, `외 N건`).
 * Under it are 이어받기, which starts a Conductor for the mission, and the
 * node's own choices for its cause; opening the row opens the mission's map.
 */
export function SidebarMissionRow({ ask, inset, actions }: { ask: MissionAsk; inset: string; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const line = { verb: "answer" as const, what: null, more: ask.more, subject: { cause: "mission_attention" as const, node: ask.node.title, reason: ask.attention.reason } };
  const name = [ask.mission.title, t("mission.attention"), ask.node.title].join(", ");
  return (
    <li data-mission-ask={ask.mission.id} data-mission-ask-node={ask.node.key} className={cn("group/row relative flex items-start gap-xs rounded-sm py-xs pr-xs text-body hover:bg-accent")} style={{ paddingLeft: inset }}>
      <Hint label={name} reveals>
        <Button
          variant="text"
          size="inline"
          aria-label={name}
          data-mission-ask-open={ask.mission.id}
          className="absolute inset-0 rounded-sm focus-visible:ring-inset"
          onClick={() => actions.openMission(ask.mission.id)}
        />
      </Hint>
      <SessionRowBody
        markClassName="pointer-events-none"
        mark={<StatusMark mark="turn" />}
        head={<>
          <span aria-hidden="true" className="pointer-events-none min-w-0 flex-1 truncate text-foreground">{ask.mission.title}</span>
          <FoldLane />
        </>}
      >
        <span aria-hidden="true" className="pointer-events-none flex min-w-0 leading-(--size-sidebar-line-detail)"><AskLine ask={line} className="flex-1" /></span>
        <span className="relative z-10 mt-xs flex flex-wrap items-center gap-xs" data-mission-ask-choices={ask.mission.id}>
          <MissionTakeOver mission={ask.mission} actions={actions} />
          <AttentionChoices mission={ask.mission} node={ask.node} attention={ask.attention} actions={actions} />
        </span>
      </SessionRowBody>
    </li>
  );
}
