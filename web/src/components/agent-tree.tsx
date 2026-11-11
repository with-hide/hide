import { ChevronDownIcon, ChevronRightIcon, EyeIcon } from "lucide-react";
import type { TFunction } from "i18next";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import type { AgentRow, AskSubject, RaiseVerb } from "../snapshot";
import type { LineParts } from "../agentRow";
import { askSubjectText } from "../askSubject";
import { DeviceChip } from "./device-chip";
import { HumanTurnCopy, type CopyText } from "./human-turn-copy";

// The parts every agent tree draws the same way (PRD agent-hierarchy-screens
// D-37 to D-43): a Needs You row's ask line, the one descendant
// mark a folded parent wears, the
// elbow rails and the chevron lane of a tree row, and the tree button. The
// core decides every value; these only draw it.

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
 * A row below the indent limit keeps the limit's column: `pass` says whether
 * the rail of its ancestor at the limit runs on past it, and its chevron lane
 * takes its parent's rail from above and hands it on down when `on`.
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
 * `limit` stands in the limit's column and hangs from its parent's chevron
 * lane, so the levels below the limit read as one rail running down.
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
 * The rails left of a tree row (D-38): a rail down each level whose ancestor
 * has a later sibling, then this row's elbow from its parent's chevron lane.
 * No tint is laid behind an opened subtree.
 */
export function TreeRails({ place }: { place: TreePlace }) {
  if (place.depth === 0) return null;
  return (
    <span aria-hidden="true" className="flex shrink-0 self-stretch" data-tree-rails={place.depth}>
      {place.rails.map((rail, index) => (
        <span key={index} className="relative w-(--size-lineage-indent) shrink-0">
          {rail ? <span className="absolute inset-y-0 left-(--size-lineage-rail-x) w-(--size-hairline) bg-lineage-rail" /> : null}
        </span>
      ))}
      {place.chain ? (
        <span className="relative w-(--size-lineage-indent) shrink-0" data-tree-pass={place.chain.pass ? "rail" : "none"}>
          {place.chain.pass ? <span className="absolute inset-y-0 left-(--size-lineage-rail-x) w-(--size-hairline) bg-lineage-rail" /> : null}
        </span>
      ) : (
        <span className="relative w-(--size-lineage-indent) shrink-0" data-tree-elbow={place.last ? "last" : "middle"}>
          <span className={cn("absolute top-0 left-(--size-lineage-rail-x) w-(--size-hairline) bg-lineage-rail", place.last ? "h-(--size-lineage-elbow-y)" : "bottom-0")} />
          <span className="absolute top-(--size-lineage-elbow-y) right-0 left-(--size-lineage-rail-x) h-(--size-hairline) bg-lineage-rail" />
        </span>
      )}
    </span>
  );
}

/**
 * The chevron lane left of a tree row's mark: a chevron where the row has
 * children, and while it is open a rail leaving it down to the children.
 * A row with none keeps the lane, crossed by its elbow when it hangs from a parent.
 */
export function TreeChevron({ name, open, onToggle, hangs, chain, disabled = false, popup = false, ...data }: { name: string; open: boolean | null; onToggle?: () => void; hangs: boolean; chain?: TreeChain; disabled?: boolean; popup?: boolean } & Record<`data-${string}`, string>) {
  // The lane spans the row; its chevron sits on line one, below the row's top padding.
  const { t } = useInterfaceTranslation();
  if (open === null || !onToggle) {
    return (
      <span aria-hidden="true" className="relative w-(--size-lineage-chevron) shrink-0 self-stretch">
        {chain ? <>
          {/* Below the indent limit the parent's rail comes down this lane and turns to the mark. */}
          <span className={cn("absolute top-0 left-(--size-lineage-rail-x) w-(--size-hairline) bg-lineage-rail", chain.on ? "bottom-0" : "h-(--size-lineage-elbow-y)")} />
          <span className="absolute top-(--size-lineage-elbow-y) right-xxs left-(--size-lineage-rail-x) h-(--size-hairline) bg-lineage-rail" />
        </> : hangs ? <span className="absolute top-(--size-lineage-elbow-y) right-xxs left-none h-(--size-hairline) bg-lineage-rail" /> : null}
      </span>
    );
  }
  const label = t(open ? "agentSessions.tree.collapse" : "agentSessions.tree.expand", { name });
  return (
    <span className="relative w-(--size-lineage-chevron) shrink-0 self-stretch">
      {chain ? <span aria-hidden="true" className="absolute top-0 left-(--size-lineage-rail-x) h-xs w-(--size-hairline) bg-lineage-rail" /> : null}
      {open || chain?.on ? <span aria-hidden="true" className="absolute top-(--size-lineage-departure-y) bottom-0 left-(--size-lineage-rail-x) w-(--size-hairline) bg-lineage-rail" /> : null}
      <button
        type="button"
        tabIndex={-1}
        aria-label={label}
        aria-expanded={popup ? undefined : open}
        aria-haspopup={popup ? "dialog" : undefined}
        disabled={disabled}
        onClick={(event) => {
          event.stopPropagation();
          onToggle();
        }}
        className="relative z-10 mt-xs flex h-(--size-sidebar-line) w-full items-center justify-center rounded-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring"
        {...data}
      >
        {open ? <ChevronDownIcon aria-hidden="true" className="size-(--size-icon-sm)" /> : <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" />}
      </button>
    </span>
  );
}
