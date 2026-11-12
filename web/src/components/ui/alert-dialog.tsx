import { AlertDialog as AlertDialogPrimitive } from "radix-ui";
import { useRef, type ComponentProps } from "react";
import { cn } from "../../lib/utils";
import { buttonVariants } from "./button";
import { useLayerOpen, useReturnFocus } from "./layer";

// A confirmation for an irreversible effect (design 6): Escape and Cancel keep
// things as they are, and the destructive button names its result.

function AlertDialog({ open, defaultOpen, onOpenChange, ...props }: ComponentProps<typeof AlertDialogPrimitive.Root>) {
  const [current, change] = useLayerOpen(open, defaultOpen, onOpenChange);
  return <AlertDialogPrimitive.Root data-slot="alert-dialog" open={current} onOpenChange={change} {...props} />;
}

function AlertDialogTrigger(props: ComponentProps<typeof AlertDialogPrimitive.Trigger>) {
  return <AlertDialogPrimitive.Trigger data-slot="alert-dialog-trigger" {...props} />;
}

function AlertDialogContent({
  className,
  initialFocus = "container",
  onOpenAutoFocus,
  onCloseAutoFocus,
  ...props
}: ComponentProps<typeof AlertDialogPrimitive.Content> & {
  /**
   * Where the keyboard lands when the dialog opens: the dialog itself (the
   * default), Radix's Cancel, or the one control marked `data-initial-focus`
   * (`action`), which only the close sheet uses, as the operator's chosen
   * Enter default (PRD close-agent-subtree D-18).
   */
  initialFocus?: "cancel" | "container" | "action";
}) {
  const surface = useRef<HTMLDivElement>(null);
  const returnFocus = useReturnFocus(true);
  return (
    <AlertDialogPrimitive.Portal>
      <AlertDialogPrimitive.Overlay data-slot="alert-dialog-overlay" className="fixed inset-0 z-50 bg-scrim" />
      <AlertDialogPrimitive.Content
        ref={surface}
        tabIndex={-1}
        data-slot="alert-dialog-content"
        className={cn(
          "fixed left-1/2 top-1/2 z-50 flex max-h-[calc(100%-var(--spacing-xxl))] w-(--size-add-device-sheet-w) max-w-[calc(100%-var(--spacing-xxl))] -translate-x-1/2 -translate-y-1/2 flex-col gap-md overflow-auto rounded-lg border border-border bg-dialog p-lg text-body text-popover-foreground [--accent:var(--dialog-hover)] [--secondary:var(--dialog-raised)] shadow-modal outline-none",
          className,
        )}
        onOpenAutoFocus={(event) => {
          onOpenAutoFocus?.(event);
          if (event.defaultPrevented || initialFocus === "cancel") return;
          event.preventDefault();
          const action = initialFocus === "action" ? surface.current?.querySelector<HTMLElement>("[data-initial-focus]") : null;
          (action ?? surface.current)?.focus();
        }}
        onCloseAutoFocus={(event) => {
          onCloseAutoFocus?.(event);
          if (!event.defaultPrevented) returnFocus(event);
        }}
        {...props}
      />
    </AlertDialogPrimitive.Portal>
  );
}

function AlertDialogHeader({ className, ...props }: ComponentProps<"div">) {
  return <div data-slot="alert-dialog-header" className={cn("flex flex-col gap-xs", className)} {...props} />;
}

function AlertDialogFooter({ className, ...props }: ComponentProps<"div">) {
  return <div data-slot="alert-dialog-footer" className={cn("flex flex-wrap justify-end gap-sm", className)} {...props} />;
}

function AlertDialogTitle({ className, ...props }: ComponentProps<typeof AlertDialogPrimitive.Title>) {
  return <AlertDialogPrimitive.Title data-slot="alert-dialog-title" className={cn("text-title font-semibold text-foreground", className)} {...props} />;
}

function AlertDialogDescription({ className, ...props }: ComponentProps<typeof AlertDialogPrimitive.Description>) {
  return <AlertDialogPrimitive.Description data-slot="alert-dialog-description" className={cn("text-body text-subtle-foreground", className)} {...props} />;
}

function AlertDialogAction({ className, variant = "destructive", ...props }: ComponentProps<typeof AlertDialogPrimitive.Action> & { variant?: "default" | "destructive" }) {
  return <AlertDialogPrimitive.Action data-slot="alert-dialog-action" className={cn(buttonVariants({ variant }), className)} {...props} />;
}

function AlertDialogCancel({ className, ...props }: ComponentProps<typeof AlertDialogPrimitive.Cancel>) {
  return <AlertDialogPrimitive.Cancel data-slot="alert-dialog-cancel" className={cn(buttonVariants({ variant: "secondary" }), className)} {...props} />;
}

export {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
};
