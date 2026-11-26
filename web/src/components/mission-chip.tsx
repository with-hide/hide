import { useContext } from "react";
import { useInterfaceTranslation } from "../i18n/client";
import { cellsFromCounts, chipLine, chipWords, conductedBy, missionCells, missionCounts, missionNumber, missionTone, missionToneOf, missionTurn, TURN_WORDS, type CellTone, type ConductorOwner } from "../missions";
import { heldBy, useDelayWords, type MissionDelay } from "./mission-hold";
import type { DeskMission } from "../desk";
import type { Mission, MissionTurn } from "../snapshot";
import { cn } from "../lib/utils";
import { CheckIcon, PauseIcon, WaypointsIcon } from "lucide-react";
import type { ComponentProps, Ref } from "react";
import { useRemaining } from "./elapsed";
import { MissionChipFace, ProgressCells, TONE_TEXT } from "./mission-tone";
import { InToolCell } from "./tool-mark";
import { useShellStore } from "../store";
import { Button } from "./ui/button";
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

/** What a hover card puts on the chip that anchors it: its handlers and states, and the ref it is placed by. */
type ChipAnchor = { anchor?: ComponentProps<"button">; ref?: Ref<HTMLButtonElement> };

/** How many lines Desk's mission chip takes: two after a paragraph, one where a table's cell or a list's item names the mission (Doop Q 가). */
export type ChipLines = 1 | 2;

/**
 * Desk's mission chip, in two lines (Doop N2 가): the mission's mark, number
 * and name, then its cells, `k/n` or its state's word, and `! 내 차례` while
 * the operator's turn waits, edged in the warning colour then. One rule
 * (`chipLine`, `web/src/missions.ts`) for a job card's mission, a mission an
 * answer names and the session beside Desk that conducts one; the sidebar's
 * Conductor row keeps its one-line chip. Only the name is cut short, its
 * whole in the hint. `lines={1}` draws the same chip on one line, in the
 * place of the mission's name in a table's cell or a list's item (Doop Q 가):
 * its cells on a fixed track so a table's rows line them up, and the
 * operator's turn as the warning edge and `!` alone, its words in the hint.
 */
function MissionChipCard({ id, title, status, done, total, cells, tone, turn, delay, lines = 2, onOpen, anchor, ref }: ChipAnchor & { id: string; title: string; status: Mission["status"]; done: number; total: number; cells: readonly CellTone[]; tone: CellTone; turn: MissionTurn | null; delay: MissionDelay | null; lines?: ChipLines; onOpen: (missionId: string) => void }) {
  const { t } = useInterfaceTranslation();
  const held = useDelayWords(delay);
  const left = useRemaining(delay?.until);
  const line = chipLine(status, { done, total }, turn !== null);
  const number = missionNumber(id);
  const word = line.word ? t(line.word) : null;
  const label = [t("mission.chip.open"), title, word, line.counts, turn ? t(TURN_WORDS[turn]) : null, held].filter(Boolean).join(" · ");
  const Mark = status === "done" ? CheckIcon : WaypointsIcon;
  const mark = <Mark aria-hidden="true" className={cn("size-(--size-icon-sm) shrink-0", TONE_TEXT[tone])} />;
  const numbered = number ? <span className="shrink-0 font-mono text-micro text-muted-foreground">{number}</span> : null;
  const name = <span className={cn("min-w-0 truncate font-medium", status === "done" ? "text-mission-done" : "text-foreground")}>{title}</span>;
  const counts = line.counts ? <span className={cn("shrink-0 font-mono font-semibold tabular-nums", status === "done" && "text-mission-done")}>{line.counts}</span> : null;
  const wordTone = status === "merge_wait" ? "text-mission-review" : status === "done" ? "text-mission-done" : "text-muted-foreground";
  const paused = delay ? <span className="flex shrink-0 items-center gap-xxs text-muted-foreground" data-mission-chip-delay="true"><PauseIcon aria-hidden="true" className="size-(--size-icon-sm)" />{left && left !== "past" ? left.text : null}</span> : null;
  const press = (event: { stopPropagation: () => void }) => {
    event.stopPropagation();
    onOpen(id);
  };
  const marks = { "aria-label": label, "data-mission-chip": id, "data-mission-chip-status": status, "data-mission-chip-tone": tone, "data-mission-chip-turn": turn ?? undefined } as const;
  const button = lines === 1 ? (
      <Button
        variant="text"
        size="inline"
        ref={ref}
        {...marks}
        data-mission-chip-lines="1"
        className={cn(
          "relative z-10 inline-flex h-auto max-w-full min-w-0 items-center gap-xs rounded-sm border bg-code-block px-xs py-px align-middle text-caption font-normal text-foreground hover:fill-hovered hover:text-foreground",
          turn ? "border-warning" : "border-hairline",
          status === "cancelled" && "opacity-(--opacity-dimmed)",
        )}
        onClick={press}
        {...anchor}
      >
        {mark}
        {numbered}
        {name}
        {line.cells ? <ProgressCells tones={cells} size="track" /> : null}
        {counts}
        {turn ? <span aria-hidden="true" className="shrink-0 font-semibold text-warning" data-mission-chip-word="true">!</span> : word ? <span className={cn("shrink-0 text-micro", wordTone)} data-mission-chip-word="true">{word}</span> : null}
        {paused}
      </Button>
  ) : (
      <Button
        variant="text"
        size="inline"
        ref={ref}
        {...marks}
        data-mission-chip-card="true"
        className={cn(
          "relative z-10 flex h-auto min-w-(--size-mission-card-min) max-w-(--size-mission-card-max) flex-col items-stretch justify-start gap-xxs rounded-sm border bg-card px-xs py-xxs text-left font-normal text-foreground hover:bg-accent hover:text-foreground",
          turn ? "border-warning" : "border-border",
          status === "cancelled" && "opacity-(--opacity-dimmed)",
        )}
        onClick={press}
        {...anchor}
      >
        <span className="flex min-w-0 items-center gap-xs text-caption">
          {mark}
          {numbered}
          {name}
        </span>
        <span className="flex min-w-0 items-center gap-xs text-micro" data-mission-chip-line="true">
          {line.cells ? <ProgressCells tones={cells} size="head" className="min-w-(--size-chip-bar)" /> : null}
          {counts}
          {word ? <span className={cn("min-w-0 truncate", turn ? "font-semibold text-warning" : wordTone)} data-mission-chip-word="true">{turn ? `! ${word}` : word}</span> : null}
          {paused ? <span className="ml-auto flex shrink-0">{paused}</span> : null}
        </span>
      </Button>
  );
  // Given the pane's hover card as its anchor, the card is the hover; else the hint says the chip.
  return anchor ? button : <Hint label={label}>{button}</Hint>;
}

/** Desk's chip of a mission as the store holds it now: its cells are the nodes', its colour the one every chip wears; `anchor` and `ref` make it the anchor of a hover card (the Conductor pane's). */
export function StoredMissionChip({ mission, lines, onOpen, anchor, ref }: ChipAnchor & { mission: Mission; lines?: ChipLines; onOpen: (missionId: string) => void }) {
  const { done, total } = missionCounts(mission);
  return <MissionChipCard id={mission.id} title={mission.title} status={mission.status} done={done} total={total} cells={missionCells(mission)} tone={missionTone(mission)} turn={missionTurn(mission)} delay={heldBy(mission)} lines={lines} onOpen={onOpen} anchor={anchor} ref={ref} />;
}

/** Desk's job card carries a mission's counts, not its nodes' states, so a mission the store does not hold draws its cells from the counts and a running one in the working colour. */
const DESK_NOT_STALLED = false;

/** A job's mission, as the job carries it (B34): the store's mission when the app holds it, else the counts the job carries. */
export function DeskMissionChip({ mission, onOpen }: { mission: DeskMission; onOpen: (missionId: string) => void }) {
  const held = useShellStore((s) => s.rest?.missions?.items.find((one) => one.id === mission.id) ?? null);
  if (held) return <StoredMissionChip mission={held} onOpen={onOpen} />;
  const delay = mission.delay ? { line: mission.delay, until: mission.delay_until_unix_ms ?? null } : null;
  const counts = { done: mission.done, total: mission.total };
  return <MissionChipCard id={mission.id} title={mission.title} status={mission.status} {...counts} cells={cellsFromCounts(counts)} tone={missionToneOf(mission.status, mission.turn !== null, DESK_NOT_STALLED)} turn={mission.turn} delay={delay} onOpen={onOpen} />;
}

/** The chip of the mission `id` as the store holds it now (a Desk answer's `@mission`, on one line in a table's cell or a list's item; the session beside Desk's head), nothing for a mission it does not hold. */
export function MissionChipOf({ id, lines, onOpen }: { id: string; lines?: ChipLines; onOpen: (missionId: string) => void }) {
  const mission = useShellStore((s) => s.rest?.missions?.items.find((one) => one.id === id) ?? null);
  return mission ? <StoredMissionChip mission={mission} lines={lines} onOpen={onOpen} /> : null;
}

function HeldMissionChip({ mission, onOpen }: { mission: Mission; onOpen: (missionId: string) => void }) {
  const { t } = useInterfaceTranslation();
  const inCell = useContext(InToolCell);
  const { key, counts } = chipWords(mission);
  const tone = missionTone(mission);
  return <MissionChipView id={mission.id} title={mission.title} words={t(key)} counts={counts} tone={inCell && tone === "hold" ? "waiting" : tone} turn={missionTurn(mission)} delay={heldBy(mission)} status={mission.status} onOpen={onOpen} />;
}

/** The sidebar's one-line chip itself, from its values alone. */
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
        className={cn("relative z-10 flex shrink-0 items-center gap-xxs rounded-xs bg-secondary px-xs text-micro outline-none hover:fill-hovered focus-visible:ring-1 focus-visible:ring-ring", TONE_TEXT[tone])}
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
