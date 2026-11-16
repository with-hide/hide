import { ChevronDownIcon, ChevronRightIcon, EyeIcon } from "lucide-react";
import type { TFunction } from "i18next";
import type { ComponentProps, ReactNode } from "react";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import type { AgentRow, AskSubject, RaiseVerb } from "../snapshot";
import type { LineParts } from "../agentRow";
import { askSubjectText } from "../askSubject";
import { DeviceChip } from "./device-chip";
import { HumanTurnCopy, type CopyText } from "./human-turn-copy";
import { Button } from "./ui/button";

// The parts every agent tree draws the same way (PRD agent-hierarchy-screens
// D-37 to D-43): a Needs You row's ask line, the one descendant
// mark a folded parent wears, the elbow rails of a tree row, its fold and the
// "N more" line. The core decides every value; these only draw it.

/** What to do, or the verb's own words when no label line or letter says it (B3). */
export function askWhat(t: TFunction<"translation">, verb: RaiseVerb, what: string | null | undefined, subject?: AskSubject | null): string {
  if (subject) return askSubjectText(t, subject);
  return what ?? t(`agentSessions.ask.${verb}`);
}

/**
 * An ask as a line's parts: what to do, with no verb leading it. The warning
 * colour and the row's `!` already say it is the operator's turn, and whether
 * it is a question, an approval or a decision is what the line itself says
 * (status-marks B5, B33; the verb words stay in notifications and hints).
 */
export function askParts(t: TFunction<"translation">, ask: NonNullable<AgentRow["state"]["ask"]>): LineParts {
  return { lead: null, text: askWhat(t, ask.verb, ask.what, ask.subject), tail: moreTail(t, ask.more) };
}

/** The count of the other asks under one root, `외 N건`, or null with none (needs-you-cleanup D-13). */
export function moreTail(t: TFunction<"translation">, more: number): string | null {
  return more > 0 ? t("agentSessions.moreAsks", { count: more }) : null;
}

/** A decision with Views linked to it says on its row how many wait to be looked at, an eye and the count; its card lists them. */
export function DecisionViewsCount({ subject, hidden }: { subject: AskSubject | null | undefined; hidden?: boolean }) {
  const { t } = useInterfaceTranslation();
  const views = subject?.cause === "mission_decision" ? (subject.views ?? 0) : 0;
  if (!views) return null;
  return (
    <span aria-hidden={hidden} className="flex shrink-0 items-center gap-xxs self-center text-muted-foreground" title={t("mission.banner.looks", { count: views })} data-ask-views={views}>
      <EyeIcon aria-hidden="true" className="size-(--size-icon-sm)" />
      {views}
    </span>
  );
}

/**
 * A Needs You row's second line: what to do, and the count of the other asks
 * waiting under the same root (B2, needs-you-cleanup B9). A person's turn
 * says its kind and place as plain text the operator can select and copy,
 * never a link, with Copy beside the place where the surface draws Done
 * (`copy`) and its device's chip (PRD ask-human B9, B10); the row's open
 * button stays under every other part of the line. `textHidden` keeps the
 * words from assistive technology on a row whose open button already says
 * them, while Copy stays reachable.
 */
export function AskLine({ ask, copy, textHidden = false, className }: { ask: NonNullable<AgentRow["state"]["ask"]>; copy?: CopyText; textHidden?: boolean; className?: string }) {
  const { t } = useInterfaceTranslation();
  const what = askWhat(t, ask.verb, ask.what, ask.subject);
  const more = moreTail(t, ask.more);
  const human = ask.subject?.cause === "human_turn" ? ask.subject : null;
  const hidden = textHidden || undefined;
  return (
    <span className={cn("group/turn flex min-w-0 items-baseline gap-xs text-caption", className)} data-ask-line={ask.verb} data-ask-subject={ask.subject?.cause}>
      <span aria-hidden={hidden} className={cn("min-w-0 truncate text-warning", human && "pointer-events-auto cursor-text select-text")} title={what} data-human-turn-place={human ? human.id : undefined}>
        {what}
      </span>
      {human && copy ? <HumanTurnCopy turn={human} copy={copy} /> : null}
      {human?.device ? <span aria-hidden={hidden} className="contents"><DeviceChip label={human.device} variant="inline" className="shrink-0 self-center" /></span> : null}
      <DecisionViewsCount subject={ask.subject} hidden={hidden} />
      {more ? <span aria-hidden={hidden} className="shrink-0 text-muted-foreground" data-ask-more={ask.more}>· {more}</span> : null}
    </span>
  );
}

/**
 * The one mark a folded parent wears for its descendants in the tree popover
 * (B17): `! N` while they raise something. A sidebar or 태스크 row draws none:
 * its root already stands in 내 차례 with the raised ask on its line
 * (needs-you-cleanup B7). A descendant that is only working draws nothing;
 * the core still computes the working mark.
 */
export function DescendantMark({ agent }: { agent: AgentRow }) {
  const { t } = useInterfaceTranslation();
  const mark = agent.descendant_mark;
  if (mark?.kind !== "raised") return null;
  return (
    <span
      role="img"
      aria-label={t("agentSessions.tree.raised", { count: mark.count })}
      data-descendant-mark={mark.kind}
      className="inline-flex shrink-0 items-center gap-xxs font-mono text-caption font-semibold text-warning"
    >
      <span aria-hidden="true">!</span>
      <span aria-hidden="true">{mark.count}</span>
    </span>
  );
}

/**
 * Where a drawn tree row sits: its depth and, per level above it, whether a
 * rail runs past it. `chain` marks a row below an indent limit (`cappedPlaces`).
 */
export type TreePlace = { depth: number; last: boolean; rails: boolean[]; chain?: TreeChain };

/**
 * A row below the indent limit stands one column past the limit: `pass` says
 * whether the rail of its ancestor at the limit runs on past it, and its elbow
 * takes the shared rail from above and hands it on down when `on`.
 */
export type TreeChain = { pass: boolean; on: boolean };

/**
 * The rails of a flattened tree in display order: each row is the last of
 * its siblings when no later row at its depth comes before a shallower one,
 * and an ancestor level carries a rail while that ancestor has a later sibling.
 */
export function treePlaces(depths: readonly number[]): TreePlace[] {
  const last = depths.map((depth, index) => {
    for (let next = index + 1; next < depths.length; next++) {
      if (depths[next]! < depth) return true;
      if (depths[next] === depth) return false;
    }
    return true;
  });
  const open: boolean[] = [];
  return depths.map((depth, index) => {
    open.length = depth;
    const rails = open.slice(1, depth);
    open[depth] = !last[index];
    return { depth, last: last[index]!, rails };
  });
}

/**
 * `treePlaces` for a panel too narrow for every level: a row deeper than
 * `limit` stands one step past the limit's column, and every deeper level in
 * that same column, hanging from one rail that comes down under the limit
 * row's mark, so the levels below the limit read as one rail running down.
 */
export function cappedPlaces(depths: readonly number[], limit: number): TreePlace[] {
  return treePlaces(depths).map((place, index) => place.depth <= limit ? place : {
    depth: limit,
    last: place.last,
    rails: place.rails.slice(0, limit - 1),
    chain: { pass: place.rails[limit - 1] ?? false, on: (depths[index + 1] ?? 0) > limit },
  });
}

/**
 * The last child's elbow: its parent's rail bends into the row on a rounded corner and stops there, rather than
 * a straight rail that runs on below the last row. Position it with the rail's left and the elbow's height.
 */
export const RAIL_ELBOW = "border-0 border-b-(length:--size-hairline) border-l-(length:--size-hairline) border-solid border-lineage-rail rounded-bl-md";

/**
 * Where a tree list's marks stand, left of the rails: past one fold column
 * (`TREE_ROOT_FOLD`), so a top row's fold stands left of its mark and its
 * children's rails start under that mark, as a 프로젝트 checkout's do.
 */
export const TREE_INSET = "calc(var(--size-lineage-chevron) + var(--spacing-xs))";

/** A top row's fold center, in the fold column `TREE_INSET` keeps. */
export const TREE_ROOT_FOLD = "calc(var(--size-lineage-chevron) / 2)";

/** Where a rail runs in its column: under the middle of the mark the column starts at, so a rail leaves a mark's middle. */
const RAIL_LEFT = "calc(var(--size-agent-mark) / 2 - var(--size-hairline) / 2)";

/** The middle of an agent row's first line, where its mark stands: the row's top padding and half a sidebar line. */
export const ROW_MID = "calc(var(--spacing-xs) + var(--size-sidebar-line) / 2)";

/** The middle of a line with no top padding of its own, such as a subagent's. */
export const LINE_MID = "calc(var(--size-sidebar-line) / 2)";

function Rail() {
  return <span className="absolute inset-y-0 w-(--size-hairline) bg-lineage-rail" style={{ left: RAIL_LEFT }} />;
}

/** A row's elbow, its parent's rail turning into the row's mark at `mid`; the last child's bends there and stops. */
function Elbow({ last, mid }: { last: boolean; mid: string }) {
  if (last) return <span className={cn("absolute top-0 right-0", RAIL_ELBOW)} style={{ left: RAIL_LEFT, height: `calc(${mid} + var(--size-hairline) / 2)` }} />;
  return <>
    <Rail />
    <span className="absolute right-0 h-(--size-hairline) bg-lineage-rail" style={{ left: RAIL_LEFT, top: `calc(${mid} - var(--size-hairline) / 2)` }} />
  </>;
}

/** How many indent columns a row's rails take: its depth, and one more below the indent limit (`cappedPlaces`). */
function columns(place: TreePlace): number {
  return place.chain ? place.depth + 1 : place.depth;
}

/**
 * The rails left of a tree row (D-38): a rail down each level whose ancestor
 * has a later sibling, then this row's elbow, the parent's rail coming down
 * under the middle of the parent's mark and turning into the middle of this
 * row's mark at `mid`. The row's mark follows the elbow directly, whether or
 * not the row opens: its fold lies over the row (`TreeFold`). Below the indent
 * limit the elbow hangs from the one rail the deeper levels share. No tint is
 * laid behind an opened subtree.
 */
export function TreeRails({ place, mid = ROW_MID, first }: { place: TreePlace; mid?: string; first?: string }) {
  if (place.depth === 0) return null;
  // The first column is `first` wide when a holder sets it (`holderLead`), every other one indent.
  const column = (index: number, child: ReactNode, data?: Record<`data-${string}`, string>) => (
    <span key={index} className="relative w-(--size-lineage-indent) shrink-0" style={index === 0 && first ? { width: first } : undefined} {...data}>{child}</span>
  );
  const at = place.rails.length;
  return (
    <span aria-hidden="true" className="flex shrink-0 self-stretch" data-tree-rails={place.depth}>
      {place.rails.map((rail, index) => column(index, rail ? <Rail /> : null))}
      {place.chain ? <>
        {column(at, place.chain.pass ? <Rail /> : null, { "data-tree-pass": place.chain.pass ? "rail" : "none" })}
        {column(at + 1, <Elbow last={!place.chain.on} mid={mid} />, { "data-tree-elbow": place.chain.on ? "middle" : "last" })}
      </> : column(at, <Elbow last={place.last} mid={mid} />, { "data-tree-elbow": place.last ? "last" : "middle" })}
    </span>
  );
}

/**
 * The tree's fold button: a chevron, right while folded and down while open,
 * that answers a press without pressing the row under it. As a popover's
 * trigger (`popup`) it says it opens a dialog instead of an expanded state.
 */
export function FoldButton({ name, open, popup = false, className, onClick, ...rest }: { name: string; open: boolean; popup?: boolean } & Omit<ComponentProps<"button">, "children">) {
  const { t } = useInterfaceTranslation();
  const label = t(open ? "agentSessions.tree.collapse" : "agentSessions.tree.expand", { name });
  return (
    <button
      type="button"
      tabIndex={-1}
      aria-label={label}
      aria-expanded={popup ? undefined : open}
      aria-haspopup={popup ? "dialog" : undefined}
      onClick={(event) => {
        event.stopPropagation();
        onClick?.(event);
      }}
      className={cn("flex size-full items-center justify-center rounded-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50", className)}
      {...rest}
    >
      {open ? <ChevronDownIcon aria-hidden="true" className="size-(--size-icon-sm)" /> : <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" />}
    </button>
  );
}

/**
 * Where a tree row's fold stands, laid over the row instead of in a lane of
 * its own, so a row's mark keeps its column whether it opens or not and the
 * rail runs from a parent's mark to its children's (docs/UI_BEHAVIOR.md, the
 * agent tree): a root's left of its mark, a child's on its own elbow, where
 * its parent's rail turns to it (`foldOnElbow`). `at` is the fold's center
 * in the row; the row positions it, so the row is `relative`.
 */
export function TreeFold({ at, children }: { at: string; children: ReactNode }) {
  return <span className="absolute top-xs z-10 h-(--size-sidebar-line) w-(--size-lineage-chevron) -translate-x-1/2" style={{ left: at }}>{children}</span>;
}

/** How far a row's first `count` columns reach: the first `first` wide when a holder sets it, the rest one indent each. */
function reach(count: number, first?: string): string {
  return count > 0 && first ? `${first} + var(--size-lineage-indent) * ${count - 1}` : `var(--size-lineage-indent) * ${count}`;
}

/** Where a tree row's mark centers: past its inset and its rails' columns. */
export function markCenter(inset: string, place?: TreePlace, first?: string): string {
  return `calc(${inset} + ${reach(place ? columns(place) : 0, first)} + var(--size-agent-mark) / 2)`;
}

/**
 * An opened row's rail leaving the foot of its mark at `at` (`markCenter`)
 * down to the row's foot, where its first child's elbow takes it on, so the
 * rail is seen to come out of the parent's mark. A row below the indent limit
 * draws none: its children hang from the rail its ancestor at the limit left.
 * A holder's row (a 프로젝트 checkout, plain folder or named group) draws the
 * same from its glyph, `mark` high and centered at `mid`, and `reach`es past
 * its foot across the gap above its list.
 */
export function TreeDeparture({ at, mid = ROW_MID, mark = "var(--size-agent-mark)", reach }: { at: string; mid?: string; mark?: string; reach?: string }) {
  return <span aria-hidden="true" data-tree-departure="" className="pointer-events-none absolute w-(--size-hairline) bg-lineage-rail" style={{ left: `calc(${at} - var(--size-hairline) / 2)`, top: `calc(${mid} + ${mark} / 2)`, bottom: reach ? `calc(-1 * ${reach})` : 0 }} />;
}

/**
 * A holder's rail passing a line that hangs nothing from it, such as an empty
 * tab's line, on its way to a later row it does hang (`holderTreeLines`).
 */
export function TreeRailPast({ at }: { at: string }) {
  return <span aria-hidden="true" data-tree-rail-past="" className="pointer-events-none absolute inset-y-0 w-(--size-hairline) bg-lineage-rail" style={{ left: `calc(${at} - var(--size-hairline) / 2)` }} />;
}

/**
 * Where a 프로젝트 holder's tree starts (a checkout, a plain folder or a named
 * group): its rows' rails half a mark left of its glyph's center at `glyph`, so
 * their first column's rail runs through it, and that first column reaching to
 * `marks`, the holder's name column, where its roots' marks stand; every column
 * after it is one indent.
 */
export function holderLead(glyph: string, marks: string): { inset: string; first: string } {
  const inset = `calc(${glyph} - var(--size-agent-mark) / 2)`;
  return { inset, first: `calc(${marks} - ${inset})` };
}

/** A tree row's fold center: on its elbow's rail, the middle of the mark above, one column short of the row's mark. */
export function foldOnElbow(inset: string, place: TreePlace, first?: string): string {
  return `calc(${inset} + ${reach(columns(place) - 1, first)} + var(--size-agent-mark) / 2)`;
}

/** The line that stands for a parent's children past the sibling limit (B12): `N개 더`, which opens them in place. */
export function TreeMoreButton({ count, onShowAll, className, ...data }: { count: number; onShowAll: () => void; className?: string } & Record<`data-${string}`, string>) {
  const { t } = useInterfaceTranslation();
  return (
    <Button variant="text" size="inline" data-tree-more={count} className={cn("min-w-0 flex-1 justify-start py-xs text-caption", className)} onClick={onShowAll} {...data}>
      {/* An agent row's first line, its chevron on the mark column, so the elbow meets it as it meets a mark. */}
      <span className="flex min-h-(--size-sidebar-line) min-w-0 items-center gap-xs">
        <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
        <span className="min-w-0 truncate">{t("agentSessions.tree.more", { count })}</span>
      </span>
    </Button>
  );
}
