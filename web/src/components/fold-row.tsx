import { ChevronDownIcon, ChevronRightIcon } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "../lib/utils";
import { RowButton } from "./ui/row-button";

/**
 * A fold in the 프로젝트 list drawn as one row (다른 체크아웃, 정리할 것, a head
 * of another machine's work): its label and count, the marks of what it
 * holds, and its chevron in the fold slot at the row's end. It is `current`
 * while it hides what the center shows, and carries the selection for it.
 */
export function FoldRow({
  label,
  count,
  expanded,
  current = false,
  badge,
  lead,
  inset = "var(--spacing-sm)",
  onToggle,
  ...data
}: {
  label: string;
  count: number;
  expanded: boolean;
  current?: boolean;
  /** The marks of what the fold holds (a `StatusBadge`), so a demand inside a fold is seen without opening it. */
  badge?: ReactNode;
  /** What stands before the label, such as a machine's letters. */
  lead?: ReactNode;
  /** Where the row's content starts; the project list's column by default. */
  inset?: string;
  onToggle: () => void;
} & Record<`data-${string}`, string>) {
  const Chevron = expanded ? ChevronDownIcon : ChevronRightIcon;
  return (
    <RowButton
      aria-expanded={expanded}
      aria-current={current ? "true" : undefined}
      className={cn("min-h-(--size-control-lg) gap-sm rounded-sm py-0 text-caption hover:text-foreground", current ? "bg-secondary text-foreground hover:bg-secondary" : "text-muted-foreground")}
      style={{ paddingLeft: inset }}
      onClick={onToggle}
      {...data}
    >
      {lead}
      <span className="min-w-0 flex-1 truncate">
        {label} {count}
      </span>
      {/* The badge keeps a project row's gap to the fold slot, so its right edge lines up with the project badges around it. */}
      <span className="flex shrink-0 items-center gap-xs">
        {badge}
        <span className="flex w-(--size-lineage-chevron) shrink-0 justify-center text-muted-foreground">
          <Chevron aria-hidden="true" className="size-(--size-icon-sm)" />
        </span>
      </span>
    </RowButton>
  );
}
