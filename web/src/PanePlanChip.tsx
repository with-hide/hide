import { CheckIcon, ChevronRightIcon, ListChecksIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type MouseEvent } from "react";
import type { Actions } from "./actions";
import { ProgressCells } from "./components/mission-tone";
import { Popover, PopoverAnchor, PopoverContent } from "./components/ui/popover";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import { paneChildren } from "./paneChildren";
import { usePaneAgents } from "./PaneHeaderBand";
import { PlanChecklist } from "./PlanChecklist";
import { PlanSummaryLine } from "./PlanMap";
import { planCells, planCount, planOfPane, planProposed, stepOf } from "./plans";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

/** How long the pointer may cross the gap between the chip and its popover before the popover closes. */
const LEAVE_MS = 150;

/**
 * The pane head's plan chip (PRD hide-plan B7, D-11, D-29; PRD
 * mission-plan-visibility B1, B3-B7), where the task chip was: the word
 * `계획`, a cell per step and `n/m`, `검토` in the warning colour while the
 * plan waits for review, a green check and `m/m` once finished, and nothing
 * for a pane with no plan. The pointer or the keyboard's focus opens the
 * checklist; a press or ⇧⌘U pins it, and a second press, ⇧⌘U or Esc closes
 * it. Opening never takes the keyboard from the terminal.
 */
export function PanePlanChip({ paneId, actions }: { paneId: string; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const plan = useShellStore((s) => planOfPane(s.rest, paneId));
  const chip = usePaneChipPopover(paneId, plan !== null);
  if (!plan) return null;
  const { close } = chip;
  const proposed = planProposed(plan);
  const finished = plan.progress.finished;
  const current = stepOf(plan, plan.progress.current);
  const label = proposed
    ? t("plan.chipReview", { count: plan.progress.total })
    : finished
      ? t("plan.chipFinished", { progress: planCount(plan) })
      : current
        ? t("plan.chipNow", { progress: planCount(plan), step: current.title })
        : t("plan.chip", { progress: planCount(plan) });
  const Mark = finished ? CheckIcon : ListChecksIcon;
  return (
    <Popover {...chip.popover}>
      <PopoverAnchor asChild>
        <button
          {...chip.anchor}
          aria-label={label}
          data-pane-plan-chip={paneId}
          data-plan-chip-state={proposed ? "review" : finished ? "finished" : "running"}
          className={cn(
            "flex h-(--size-sidebar-line-detail) shrink-0 items-center gap-xxs rounded-xs border border-border bg-background px-xs text-micro text-muted-foreground outline-none hover:bg-popover hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring data-[state=open]:border-ring data-[state=open]:text-foreground",
            proposed && "border-mission-hold text-mission-hold data-[state=open]:border-mission-hold data-[state=open]:text-mission-hold",
            finished && "text-mission-done data-[state=open]:text-mission-done",
          )}
        >
          <Mark aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
          <span className="whitespace-nowrap">{proposed ? t("plan.review") : t("plan.title")}</span>
          <ProgressCells tones={planCells(plan)} size="chip" />
          {proposed ? null : <span className="font-mono tabular-nums">{planCount(plan)}</span>}
        </button>
      </PopoverAnchor>
      <PopoverContent
        side="bottom"
        align="start"
        collisionPadding={8}
        className="max-h-(--radix-popover-content-available-height) w-(--size-pr-popover) overflow-y-auto p-0 text-caption"
        data-pane-plan={paneId}
        {...chip.content}
      >
        <div className="flex min-w-0 flex-col [word-break:keep-all]">
          <div className="flex min-w-0 items-center gap-xs px-sm pt-sm">
            <span className="min-w-0 flex-1"><PlanSummaryLine plan={plan} /></span>
            <button
              type="button"
              className="flex shrink-0 items-center gap-xxs rounded-xs px-xs py-xxs text-primary outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring"
              onClick={() => {
                close();
                actions.openPlan(plan.session);
              }}
              data-pane-plan-large="true"
            >
              {t("plan.large")}
              <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" />
            </button>
          </div>
          <div className="px-xs py-sm">
            <PlanChecklist plan={plan} revealCurrent />
          </div>
          <Delegated paneId={paneId} />
        </div>
      </PopoverContent>
    </Popover>
  );
}

/**
 * What a pane head chip with a popover shares (PRD hide-plan B7,
 * mission-pane-chip D-01): the pointer or the keyboard's focus opens it, a
 * press or the Show plan chord pins it through `ui.planPane`, and a second
 * press, the chord or Esc closes it. Opening never takes the keyboard from
 * the terminal. `present` is false while the pane has nothing to show, and a
 * pin outlives that only as long as the pane does.
 */
export function usePaneChipPopover(paneId: string, present: boolean) {
  const pinned = useUiStore((s) => s.planPane === paneId);
  const setPinned = useUiStore((s) => s.setPlanPane);
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const leave = useRef<number | null>(null);
  const chipRef = useRef<HTMLButtonElement>(null);
  useEffect(() => () => {
    if (leave.current !== null) window.clearTimeout(leave.current);
  }, []);
  useEffect(() => {
    if (present) return;
    if (pinned) setPinned(null);
    setHovered(false);
    setFocused(false);
  }, [present, pinned, setPinned]);
  const open = present && (pinned || hovered || focused);
  const enter = () => {
    if (leave.current !== null) window.clearTimeout(leave.current);
    leave.current = null;
    setHovered(true);
  };
  const exit = () => {
    if (leave.current !== null) window.clearTimeout(leave.current);
    leave.current = window.setTimeout(() => setHovered(false), LEAVE_MS);
  };
  const close = () => {
    setPinned(null);
    setHovered(false);
    setFocused(false);
  };
  return {
    open,
    close,
    popover: { open, onOpenChange: (next: boolean) => (next ? undefined : close()), modal: false },
    anchor: {
      ref: chipRef,
      type: "button" as const,
      "aria-expanded": open,
      "aria-pressed": pinned,
      "data-state": open ? "open" : "closed",
      onMouseDown: (event: MouseEvent) => event.preventDefault(),
      // A second press closes the map outright, though the pointer still rests on the chip.
      onClick: () => (pinned ? close() : setPinned(paneId)),
      onPointerEnter: enter,
      onPointerLeave: exit,
      onFocus: () => setFocused(true),
      onBlur: () => setFocused(false),
    },
    content: {
      onOpenAutoFocus: (event: Event) => event.preventDefault(),
      onCloseAutoFocus: (event: Event) => event.preventDefault(),
      onPointerEnter: enter,
      onPointerLeave: exit,
      // The chip is outside the popover, so its own press would dismiss
      // the popover before its click toggles the pin; the click alone decides.
      onInteractOutside: (event: { target: EventTarget | null; preventDefault: () => void }) => {
        if (event.target instanceof Node && chipRef.current?.contains(event.target)) event.preventDefault();
      },
      // Esc closes a pinned map and stops there, so an agent CLI in the
      // focused terminal is not interrupted by it; a map the pointer only
      // shows leaves Esc to the terminal.
      onEscapeKeyDown: (event: KeyboardEvent) => {
        if (!pinned && !focused) return event.preventDefault();
        event.stopPropagation();
        close();
      },
    },
  };
}

/** `맡긴 일 N · 끝남 k`: the pane's direct children, by the core's counts. */
function Delegated({ paneId }: { paneId: string }) {
  const { t } = useInterfaceTranslation();
  const agents = usePaneAgents();
  const rest = useShellStore((s) => s.rest);
  const children = useMemo(() => paneChildren(paneId, agents, rest), [paneId, agents, rest]);
  if (!children) return null;
  const done = children.parts.find((part) => part.kind === "result")?.count ?? 0;
  return (
    <p className="flex items-baseline gap-xs px-sm pb-sm text-micro font-medium text-muted-foreground" data-pane-plan-children={children.children.length}>
      {t("sessionWork.delegatedCount", { count: children.children.length })}
      {done > 0 ? <span className="font-normal">· {t("plan.childrenDone", { count: done })}</span> : null}
    </p>
  );
}
