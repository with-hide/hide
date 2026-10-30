import { useInterfaceTranslation } from "../i18n/client";
import { chipWords, conductedBy, missionDeciding, missionTone, type CellTone, type ConductorOwner } from "../missions";
import { cn } from "../lib/utils";
import { TONE_TEXT } from "./mission-tone";
import { useShellStore } from "../store";
import { Hint } from "./ui/tooltip";

/**
 * The Conductor row's mission mark (PRD software-factory-conductor B14): the
 * mission it conducts as `미션 n/m`, its plan or its merge wait; pressing it
 * opens the map in the lane's 보기 slot. Its words stay; its colour is the
 * map's state colour (PRD mission-plan-visibility B35, D-23), and a decision
 * waiting adds `!` before them (needs-you-cleanup B5). A row that
 * conducts nothing draws nothing, so no other row moves.
 */
export function MissionChip({ agent, onOpen }: { agent: ConductorOwner; onOpen: (missionId: string) => void }) {
  const { t } = useInterfaceTranslation();
  const mission = useShellStore((s) => conductedBy(s.rest, agent));
  if (!mission) return null;
  const { key, counts } = chipWords(mission);
  return <MissionChipView id={mission.id} title={mission.title} words={t(key)} counts={counts} tone={missionTone(mission)} deciding={missionDeciding(mission)} status={mission.status} onOpen={onOpen} />;
}

/**
 * The chip itself, from its values alone: Desk's cards and its thread draw a
 * mission they were handed, not one the store holds (PRD
 * session-docs-and-assistant B34), in this same shape.
 */
export function MissionChipView({ id, title, words, counts, tone, deciding, status, onOpen }: {
  id: string;
  title: string;
  /** The chip's words, such as `미션`. */
  words: string;
  /** `k/n`, or nothing while the mission plans or waits. */
  counts: string | null;
  tone: CellTone;
  deciding: boolean;
  status?: string;
  onOpen: (missionId: string) => void;
}) {
  const { t } = useInterfaceTranslation();
  const label = [t("mission.chip.open"), title, deciding ? t("mission.askDecision") : null].filter(Boolean).join(" · ");
  return (
    <Hint label={label}>
      <button
        type="button"
        aria-label={label}
        data-mission-chip={id}
        data-mission-chip-status={status}
        data-mission-chip-tone={tone}
        data-mission-chip-decision={deciding ? "true" : undefined}
        className={cn("relative z-10 flex shrink-0 items-center gap-xxs rounded-xs bg-secondary px-xs text-micro outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring", TONE_TEXT[tone])}
        onClick={(event) => {
          event.stopPropagation();
          onOpen(id);
        }}
      >
        {deciding ? <span aria-hidden="true" className="font-semibold">!</span> : null}
        <span>{words}</span>
        {counts ? <span className="font-mono tabular-nums">{counts}</span> : null}
      </button>
    </Hint>
  );
}
