import type { ComponentProps } from "react";
import { cn } from "../../lib/utils";

function Kbd({ className, ...props }: ComponentProps<"kbd">) {
  return (
    <kbd
      data-slot="kbd"
      className={cn(
        "pointer-events-none inline-flex h-(--size-keycap-height) w-fit min-w-(--size-keycap-height) select-none items-center justify-center gap-xxs rounded-xs border border-keycap-line px-xs font-sans text-caption font-medium text-subtle-foreground [&_svg:not([class*='size-'])]:size-(--size-icon-sm)",
        className,
      )}
      {...props}
    />
  );
}

function KbdGroup({ className, ...props }: ComponentProps<"kbd">) {
  return <kbd data-slot="kbd-group" className={cn("inline-flex items-center gap-xxs", className)} {...props} />;
}

export { Kbd, KbdGroup };
