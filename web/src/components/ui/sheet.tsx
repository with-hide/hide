import { Dialog as SheetPrimitive } from "radix-ui";
import { useRef, type ComponentProps } from "react";
import { cn } from "../../lib/utils";
import { useLayerOpen, useReturnFocus } from "./layer";

// A panel that slides over one edge of the window (shadcn Sheet), on the same
// Escape layers and focus return as Dialog.

function Sheet({ open, defaultOpen, onOpenChange, ...props }: ComponentProps<typeof SheetPrimitive.Root>) {
  const [current, change] = useLayerOpen(open, defaultOpen, onOpenChange);
  return <SheetPrimitive.Root data-slot="sheet" open={current} onOpenChange={change} {...props} />;
}

function SheetTrigger(props: ComponentProps<typeof SheetPrimitive.Trigger>) {
  return <SheetPrimitive.Trigger data-slot="sheet-trigger" {...props} />;
}

function SheetClose(props: ComponentProps<typeof SheetPrimitive.Close>) {
  return <SheetPrimitive.Close data-slot="sheet-close" {...props} />;
}

function SheetContent({
  className,
  side = "right",
  onCloseAutoFocus,
  returnFocusTo,
  ...props
}: ComponentProps<typeof SheetPrimitive.Content> & { side?: "top" | "right" | "bottom" | "left"; returnFocusTo?: () => HTMLElement | null }) {
  const surface = useRef<HTMLDivElement>(null);
  const returnFocus = useReturnFocus(true, returnFocusTo ? { target: returnFocusTo, within: () => surface.current } : undefined);
  return (
    <SheetPrimitive.Portal>
      <SheetPrimitive.Overlay data-slot="sheet-overlay" className="fixed inset-0 z-50 bg-scrim" />
      <SheetPrimitive.Content
        ref={surface}
        data-slot="sheet-content"
        data-side={side}
        className={cn(
          "fixed z-50 flex flex-col bg-dialog text-body text-popover-foreground [--accent:var(--dialog-hover)] [--secondary:var(--dialog-raised)] shadow-modal outline-none",
          side === "right" && "inset-y-0 right-0 w-(--size-panel-ideal) max-w-full border-l border-border",
          side === "left" && "inset-y-0 left-0 w-(--size-panel-ideal) max-w-full border-r border-border",
          side === "top" && "inset-x-0 top-0 h-auto border-b border-border",
          side === "bottom" && "inset-x-0 bottom-0 h-auto border-t border-border",
          className,
        )}
        onCloseAutoFocus={(event) => {
          onCloseAutoFocus?.(event);
          if (!event.defaultPrevented) returnFocus(event);
        }}
        {...props}
      />
    </SheetPrimitive.Portal>
  );
}

function SheetHeader({ className, ...props }: ComponentProps<"div">) {
  return <div data-slot="sheet-header" className={cn("flex flex-col gap-xs p-md", className)} {...props} />;
}

function SheetFooter({ className, ...props }: ComponentProps<"div">) {
  return <div data-slot="sheet-footer" className={cn("mt-auto flex flex-col gap-xs p-md", className)} {...props} />;
}

function SheetTitle({ className, ...props }: ComponentProps<typeof SheetPrimitive.Title>) {
  return <SheetPrimitive.Title data-slot="sheet-title" className={cn("text-title font-semibold text-foreground", className)} {...props} />;
}

function SheetDescription({ className, ...props }: ComponentProps<typeof SheetPrimitive.Description>) {
  return <SheetPrimitive.Description data-slot="sheet-description" className={cn("text-body text-subtle-foreground", className)} {...props} />;
}

export { Sheet, SheetClose, SheetContent, SheetDescription, SheetFooter, SheetHeader, SheetTitle, SheetTrigger };
