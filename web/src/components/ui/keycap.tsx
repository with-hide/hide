import type { ComponentProps } from "react";
import { cn } from "../../lib/utils";

/**
 * The number an agent row or a device tile carries while a modifier hold reveals it
 * (PRD electron-digit-shortcuts-hints D-03, B5, B7): a popover-colored cap
 * with a border, a small shadow and one mono digit, floating at the top
 * right of the item it numbers. It is absolutely positioned, so it never
 * moves a row's title, its time or its chevron, or a tile's marks, and
 * it takes no pointer, so a click lands on what it covers. The digit is
 * decoration for the chord the sheet already names, so it is hidden from
 * assistive technology.
 */
function Keycap({ number, className, ...props }: ComponentProps<"kbd"> & { number: number }) {
  return (
    <kbd
      data-slot="keycap"
      data-keycap={number}
      aria-hidden="true"
      className={cn(
        "pointer-events-none absolute right-xxs top-xxs z-10 inline-flex h-(--size-keycap-height) min-w-(--size-keycap-height) select-none items-center justify-center rounded-xs border border-border bg-popover px-xxs font-mono text-caption font-medium leading-none text-popover-foreground shadow-sm",
        className,
      )}
      {...props}
    >
      {number}
    </kbd>
  );
}

export { Keycap };
