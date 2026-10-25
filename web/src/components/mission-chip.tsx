import { useInterfaceTranslation } from "../i18n/client";
import { chipWords, conductedBy, missionTone, type ConductorOwner } from "../missions";
import { cn } from "../lib/utils";
import { TONE_TEXT } from "./mission-tone";
import { useShellStore } from "../store";
import { Hint } from "./ui/tooltip";

/**
 * The Conductor row's mission mark (PRD software-factory-conductor B14): the
 * mission it conducts as `미션 n/m`, its plan or its merge wait; pressing it
 * opens the map in the lane's 보기 slot. Its words stay; its colour is the
 * map's state colour (PRD mission-plan-visibility B35, D-23). A row that
 * conducts nothing draws nothing, so no other row moves.
 */
export function MissionChip({ agent, onOpen }: { agent: ConductorOwner; onOpen: (missionId: string) => void }) {
  const { t } = useInterfaceTranslation();
  const mission = useShellStore((s) => conductedBy(s.rest, agent));
  if (!mission) return null;
  const { key, counts } = chipWords(mission);
  const tone = missionTone(mission);
  const label = `${t("mission.chip.open")} · ${mission.title}`;
  return (
    <Hint label={label}>
      <button
        type="button"
        aria-label={label}
        data-mission-chip={mission.id}
        data-mission-chip-status={mission.status}
        data-mission-chip-tone={tone}
        className={cn("relative z-10 flex shrink-0 items-center gap-xxs rounded-xs bg-secondary px-xs text-micro outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring", TONE_TEXT[tone])}
        onClick={(event) => {
          event.stopPropagation();
          onOpen(mission.id);
        }}
      >
        <span>{t(key)}</span>
        {counts ? <span className="font-mono tabular-nums">{counts}</span> : null}
      </button>
    </Hint>
  );
}
