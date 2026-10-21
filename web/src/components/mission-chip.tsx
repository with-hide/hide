import { useInterfaceTranslation } from "../i18n/client";
import { chipWords, conductedBy, type ConductorOwner } from "../missions";
import { useShellStore } from "../store";
import { Hint } from "./ui/tooltip";

/**
 * The Conductor row's mission mark (PRD software-factory-conductor B14): the
 * mission it conducts as `미션 n/m`, its plan or its merge wait; pressing it
 * opens the map in the lane's 보기 slot. A row that conducts nothing draws
 * nothing, so no other row moves.
 */
export function MissionChip({ agent, onOpen }: { agent: ConductorOwner; onOpen: (missionId: string) => void }) {
  const { t } = useInterfaceTranslation();
  const mission = useShellStore((s) => conductedBy(s.rest, agent));
  if (!mission) return null;
  const { key, counts } = chipWords(mission);
  const label = `${t("mission.chip.open")} · ${mission.title}`;
  return (
    <Hint label={label}>
      <button
        type="button"
        aria-label={label}
        data-mission-chip={mission.id}
        data-mission-chip-status={mission.status}
        className="relative z-10 flex shrink-0 items-center gap-xxs rounded-xs bg-secondary px-xs text-micro text-subtle-foreground outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring"
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
