import type { HTMLAttributes, KeyboardEvent, ReactNode } from "react";
import { cn } from "../lib/utils";
import { Hint } from "./ui/tooltip";

/**
 * One session or milestone as a row in the shape of the sidebar's agent row
 * (PRD pr-card-and-session-record-redesign D-07, D-13, B7-B9, B16): the mark
 * at the left, then the title, its chips, and the right slot holding the
 * time, and under the title a caption line. The PR card's timeline and the
 * session record's children both draw this one part, so the operator learns
 * one shape.
 *
 * Nothing grows or moves under the pointer or the keyboard: `reveal` takes the
 * right slot's place while the pointer or focus is on the row (an ended
 * session's `대화 보기 · 이어서 하기`, a live one's `pane으로 가기`, a child's
 * `기록 보기`) and `right` hides meanwhile, as the sidebar rows do with
 * `REVEALED_CONTROL`. `below` is always drawn, for the `답하러 가기` an asking
 * row keeps (B8).
 */
export function SessionRow({
  mark,
  title,
  titleClassName,
  hint,
  chips,
  right,
  reveal,
  line,
  below,
  onPress,
  pressLabel,
  className,
  children,
  ...rest
}: {
  /** The status mark or the milestone square, in its colour. */
  mark: ReactNode;
  title: string;
  titleClassName?: string;
  /** The row's tooltip: the request, the parent, the device, what the line leaves out. */
  hint?: string | null;
  chips?: ReactNode;
  /** The right end of the first line: the time. */
  right?: ReactNode;
  /** What stands in the right slot while the pointer or the keyboard is on the row. */
  reveal?: ReactNode;
  /** The caption under the title; `wrap` lets a question wrap between words instead of truncating (B20). */
  line?: { text: string; className?: string; wrap?: boolean } | null;
  /** Under the second line, always shown. */
  below?: ReactNode;
  /** Pressing the row itself; the row is then a button and `pressLabel` names it. */
  onPress?: () => void;
  pressLabel?: string;
  children?: ReactNode;
} & Omit<HTMLAttributes<HTMLLIElement>, "title">) {
  const onKeyDown = (event: KeyboardEvent<HTMLLIElement>) => {
    if (!onPress || event.target !== event.currentTarget) return;
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      onPress();
    }
  };
  const row = (
    <li
      {...rest}
      role={onPress ? "button" : undefined}
      tabIndex={onPress || reveal ? 0 : undefined}
      aria-label={pressLabel ?? rest["aria-label"]}
      onClick={onPress ? (event) => {
        // A button inside the row is its own press.
        if ((event.target as HTMLElement).closest("button, a, [data-pr-mark-press]") !== null && event.target !== event.currentTarget) return;
        onPress();
      } : undefined}
      onKeyDown={onKeyDown}
      className={cn("group/row relative flex min-w-0 items-start gap-xs rounded-sm py-xs pr-xs pl-xs text-body outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring", onPress && "cursor-pointer", className)}
      data-session-row="true"
    >
      <span className="flex min-h-(--size-sidebar-line) shrink-0 items-center gap-xs" aria-hidden="true">{mark}</span>
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="flex min-h-(--size-sidebar-line) min-w-0 items-center gap-xs">
          <span className={cn("min-w-0 flex-auto truncate", titleClassName ?? "text-foreground")} data-session-row-title="true">{title}</span>
          {chips}
          {reveal ? (
            <>
              <span className={cn("flex shrink-0 items-center gap-xxs", "hidden group-focus-within/row:flex group-hover/row:flex hoverless:flex")} data-session-row-reveal="true">{reveal}</span>
              {right ? <span className="flex shrink-0 items-center group-focus-within/row:hidden group-hover/row:hidden hoverless:hidden" data-session-row-right="true">{right}</span> : null}
            </>
          ) : right ? <span className="flex shrink-0 items-center" data-session-row-right="true">{right}</span> : null}
        </span>
        {line ? <span className={cn("text-caption leading-(--size-sidebar-line-detail)", line.wrap ? "whitespace-normal break-keep [overflow-wrap:anywhere]" : "truncate", line.className ?? "text-muted-foreground")} data-session-row-line="true">{line.text}</span> : null}
        {below ? <span className="flex min-w-0 flex-wrap items-center gap-xs pt-xxs">{below}</span> : null}
        {children}
      </span>
    </li>
  );
  return hint ? <Hint label={hint} reveals>{row}</Hint> : row;
}

/** A milestone's mark: a small square in the event's colour, where a session row has its status mark. */
export function MilestoneMark({ className }: { className: string }) {
  return (
    <span aria-hidden="true" className={cn("inline-flex size-(--size-agent-mark) shrink-0 items-center justify-center", className)} data-milestone-mark="true">
      <span className="size-(--size-status-mark) rounded-xs bg-current" />
    </span>
  );
}
