import { ChevronRightIcon } from "lucide-react";
import type { ComponentProps } from "react";
import { cn } from "../lib/utils";
import { RowButton } from "./ui/row-button";

/**
 * The last row of a pane head chip's popover, the one place that says what to
 * press to open the whole thing: `지도 열기 ›` under a mission, `크게 보기 ›`
 * under a plan's steps. It spans the popover's width above a line, so the
 * action reads as its own row and never as a word inside the head.
 */
export function PopoverOpenRow({ label, className, ...props }: { label: string } & Omit<ComponentProps<typeof RowButton>, "children">) {
  return (
    <RowButton className={cn("rounded-none border-t border-border px-sm py-sm text-foreground", className)} {...props}>
      <span className="min-w-0 flex-1">{label}</span>
      <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
    </RowButton>
  );
}
