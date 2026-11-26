import { useContext } from "react";
import { useInterfaceTranslation } from "../i18n/client";
import { chipStateWords, chipWords, conductedBy, missionCounts, missionNumber, missionTone, missionToneOf, missionTurn, TURN_WORDS, type CellTone, type ConductorOwner } from "../missions";
import { heldBy, useDelayWords, type MissionDelay } from "./mission-hold";
import type { DeskMission } from "../desk";
import type { Mission, MissionTurn } from "../snapshot";
import { cn } from "../lib/utils";
import { MissionChipFace, TONE_TEXT } from "./mission-tone";
import { InToolCell } from "./tool-mark";
import { useShellStore } from "../store";
import { Hint } from "./ui/tooltip";

/**
 * The Conductor row's mission mark (PRD software-factory-conductor B14): the
 * mission it conducts as `미션 n/m`, its plan or its merge wait; pressing it
 * opens the map as a card tab on a desk. Its face is the mission icon and,
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
  const mission = useShellStore((s) => conductedBy(s.rest, agent));
  return mission ? <HeldMissionChip mission={mission} onOpen={onOpen} /> : null;
}

/**
 * Desk's mission chip, in the Conductor pane's chip's words: the mission's
 * number, its state's word and `k/n` (`M1 · 0/2`), one rule for the carried
 * mission of a job card and for a mission the store holds now (`MissionChipOf`).
 * A mission whose id has no number keeps its word.
 */
function DeskChip({ id, title, status, done, total, planned, tone, turn, delay, onOpen }: { id: string; title: string; status: Mission["status"]; done: number; total: number; planned: boolean; tone: CellTone; turn: MissionTurn | null; delay: MissionDelay | null; onOpen: (missionId: string) => void }) {
  const { t } = useInterfaceTranslation();
  const { key, counts } = chipStateWords(status, { done, total }, planned);
  const number = missionNumber(id);
  return <MissionChipView id={id} title={title} words={t(key)} counts={counts} number={number} word={number && key === "mission.chip" ? null : t(key)} tone={tone} turn={turn} delay={delay} status={status} onOpen={onOpen} />;
}

/** Desk's job card carries a mission's counts, not its nodes' states, so its chip draws a running mission in the working colour; the sidebar's and the pane's chips read the stall. */
const DESK_NOT_STALLED = false;

/** A job's mission, as the job carries it (B34). */
export function DeskMissionChip({ mission, onOpen }: { mission: DeskMission; onOpen: (missionId: string) => void }) {
  const delay = mission.delay ? { line: mission.delay, until: mission.delay_until_unix_ms ?? null } : null;
  return <DeskChip id={mission.id} title={mission.title} status={mission.status} done={mission.done} total={mission.total} planned={mission.total > 0} tone={missionToneOf(mission.status, mission.turn !== null, DESK_NOT_STALLED)} turn={mission.turn} delay={delay} onOpen={onOpen} />;
}

/** The chip of the mission `id` as the store holds it now (a Desk answer's `@mission`), nothing for a mission it does not hold. */
export function MissionChipOf({ id, onOpen }: { id: string; onOpen: (missionId: string) => void }) {
  const mission = useShellStore((s) => s.rest?.missions?.items.find((one) => one.id === id) ?? null);
  if (!mission) return null;
  const { done, total } = missionCounts(mission);
  return <DeskChip id={mission.id} title={mission.title} status={mission.status} done={done} total={total} planned={mission.nodes.length > 0} tone={missionTone(mission)} turn={missionTurn(mission)} delay={heldBy(mission)} onOpen={onOpen} />;
}

function HeldMissionChip({ mission, onOpen }: { mission: Mission; onOpen: (missionId: string) => void }) {
  const { t } = useInterfaceTranslation();
  const inCell = useContext(InToolCell);
  const { key, counts } = chipWords(mission);
  const tone = missionTone(mission);
  return <MissionChipView id={mission.id} title={mission.title} words={t(key)} counts={counts} tone={inCell && tone === "hold" ? "waiting" : tone} turn={missionTurn(mission)} delay={heldBy(mission)} status={mission.status} onOpen={onOpen} />;
}

/**
 * The chip itself, from its values alone: Desk's cards and its thread draw a
 * mission they were handed, not one the store holds (PRD
 * session-docs-and-assistant B34), in this same shape.
 */
export function MissionChipView({ id, title, words, counts, number = null, word = null, tone, turn, delay, status, onOpen }: {
  id: string;
  title: string;
  /** The chip's words, such as `미션`, said by its hint and name. */
  words: string;
  /** `k/n`, or nothing while the mission plans or waits. */
  counts: string | null;
  /** Desk's chip: the mission's number and the state's word drawn beside the count (`MissionChipFace`). */
  number?: string | null;
  word?: string | null;
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
        <MissionChipFace turn={turn !== null} delay={delay !== null} status={status} counts={counts} number={number} word={word} />
      </button>
    </Hint>
  );
}
