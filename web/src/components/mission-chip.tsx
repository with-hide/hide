import { useContext } from "react";
import { useInterfaceTranslation } from "../i18n/client";
import { chipWords, conductedBy, missionTone, missionTurn, TURN_WORDS, type CellTone, type ConductorOwner } from "../missions";
import { heldBy, useDelayWords, type MissionDelay } from "./mission-hold";
import type { MissionTurn } from "../snapshot";
import { cn } from "../lib/utils";
import { MissionChipFace, TONE_TEXT } from "./mission-tone";
import { InToolCell } from "./tool-mark";
import { useShellStore } from "../store";
import { Hint } from "./ui/tooltip";

/**
 * The Conductor row's mission mark (PRD software-factory-conductor B14): the
 * mission it conducts as `미션 n/m`, its plan or its merge wait; pressing it
 * opens the map in the lane's 보기 slot. Its face is the mission icon and,
 * while it runs, `n/m`; its words move to its hint and name. Its colour is the
 * map's state colour (PRD mission-plan-visibility B35, D-23); the operator's
 * turn, a decision or a node raised to them, adds `!` before it
 * (needs-you-cleanup B5), and a mission delay a grey pause. In an external
 * tool's cell that turn is the tool's, so the chip keeps its `!` in the grey
 * of the cell's count, never in the operator's colour (PRD
 * hide-external-control D-46, operator decision 2026-11-14). A row that
 * conducts nothing draws nothing, so no other row moves.
 */
export function MissionChip({ agent, onOpen }: { agent: ConductorOwner; onOpen: (missionId: string) => void }) {
  const { t } = useInterfaceTranslation();
  const inCell = useContext(InToolCell);
  const mission = useShellStore((s) => conductedBy(s.rest, agent));
  if (!mission) return null;
  const { key, counts } = chipWords(mission);
  const tone = missionTone(mission);
  return <MissionChipView id={mission.id} title={mission.title} words={t(key)} counts={counts} tone={inCell && tone === "hold" ? "waiting" : tone} turn={missionTurn(mission)} delay={heldBy(mission)} status={mission.status} onOpen={onOpen} />;
}

/**
 * The chip itself, from its values alone: Desk's cards and its thread draw a
 * mission they were handed, not one the store holds (PRD
 * session-docs-and-assistant B34), in this same shape.
 */
export function MissionChipView({ id, title, words, counts, tone, turn, delay, status, onOpen }: {
  id: string;
  title: string;
  /** The chip's words, such as `미션`, said by its hint and name. */
  words: string;
  /** `k/n`, or nothing while the mission plans or waits. */
  counts: string | null;
  tone: CellTone;
  /** The operator's turn, marked `!`. */
  turn: MissionTurn | null;
  /** The mission delay holding it, marked by a grey pause and said by its name with how long it still holds. */
  delay: MissionDelay | null;
  status?: string;
  onOpen: (missionId: string) => void;
}) {
  const { t } = useInterfaceTranslation();
  const held = useDelayWords(delay);
  const label = [t("mission.chip.open"), title, words, counts, turn ? t(TURN_WORDS[turn]) : null, held].filter(Boolean).join(" · ");
  return (
    <Hint label={label}>
      <button
        type="button"
        aria-label={label}
        data-mission-chip={id}
        data-mission-chip-status={status}
        data-mission-chip-tone={tone}
        data-mission-chip-turn={turn ?? undefined}
        data-mission-chip-delay={delay ? "true" : undefined}
        className={cn("relative z-10 flex shrink-0 items-center gap-xxs rounded-xs bg-secondary px-xs text-micro outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring", TONE_TEXT[tone])}
        onClick={(event) => {
          event.stopPropagation();
          onOpen(id);
        }}
      >
        <MissionChipFace turn={turn !== null} delay={delay !== null} status={status} counts={counts} />
      </button>
    </Hint>
  );
}
