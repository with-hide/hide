import type { ComponentProps } from "react";
import { cn } from "../../lib/utils";

const row = "rounded-xs outline-none transition-colors hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-(--opacity-disabled)";

/**
 * A whole row that is one action (open, pick, expand): it spans its line,
 * reads left to right and lights on hover. Its content is laid out by the
 * caller; a row with its own buttons inside uses RowOverlayButton instead,
 * since a button cannot hold another.
 */
function RowButton({ className, type = "button", ...props }: ComponentProps<"button">) {
  return <button data-slot="row-button" type={type} className={cn("flex w-full min-w-0 items-center gap-xs px-xs py-xs text-left", row, className)} {...props} />;
}

/**
 * The button of a row whose content keeps controls of its own: it lies behind
 * the content and fills the row, so a press anywhere else is the row's. Put it
 * first in a `relative` container and give the content `pointer-events-none
 * relative`, with `pointer-events-auto` on each control inside; it has no
 * text, so `aria-label` names the row's action.
 */
function RowOverlayButton({ className, type = "button", ...props }: ComponentProps<"button"> & { "aria-label": string }) {
  return <button data-slot="row-overlay-button" type={type} className={cn("absolute inset-0", row, className)} {...props} />;
}

export { RowButton, RowOverlayButton };
