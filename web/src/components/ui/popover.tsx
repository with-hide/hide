import { Popover as PopoverPrimitive } from "radix-ui";
import type { ComponentProps } from "react";
import { cn } from "../../lib/utils";
import { useLayerOpen } from "./layer";

/**
 * `escape` false leaves Escape to whatever has the keyboard, for a popover the
 * pointer only shows over a terminal.
 */
function Popover({ open, defaultOpen, onOpenChange, escape = true, ...props }: ComponentProps<typeof PopoverPrimitive.Root> & { escape?: boolean }) {
  const [current, change] = useLayerOpen(open, defaultOpen, onOpenChange, false, escape);
  return <PopoverPrimitive.Root data-slot="popover" open={current} onOpenChange={change} {...props} />;
}

function PopoverTrigger(props: ComponentProps<typeof PopoverPrimitive.Trigger>) {
  return <PopoverPrimitive.Trigger data-slot="popover-trigger" {...props} />;
}

function PopoverAnchor(props: ComponentProps<typeof PopoverPrimitive.Anchor>) {
  return <PopoverPrimitive.Anchor data-slot="popover-anchor" {...props} />;
}

function PopoverContent({ className, align = "center", sideOffset = 4, ...props }: ComponentProps<typeof PopoverPrimitive.Content>) {
  return (
    <PopoverPrimitive.Portal>
      <PopoverPrimitive.Content
        data-slot="popover-content"
        align={align}
        sideOffset={sideOffset}
        className={cn("z-50 w-(--size-pr-popover) max-w-(--radix-popover-content-available-width) rounded-md border border-border bg-popover p-sm text-body text-popover-foreground shadow-lg outline-none", className)}
        {...props}
      />
    </PopoverPrimitive.Portal>
  );
}

export { Popover, PopoverAnchor, PopoverContent, PopoverTrigger };
