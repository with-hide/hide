import { Dialog as DialogPrimitive } from "radix-ui";
import { XIcon } from "lucide-react";
import { useRef, type ComponentProps } from "react";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import { useLayerOpen, useReturnFocus } from "./layer";

/** Radix's dialog root, with its open state joined to the shell's Escape layers. */
function Dialog({ open, defaultOpen, onOpenChange, baseEscape = false, ...props }: ComponentProps<typeof DialogPrimitive.Root> & { baseEscape?: boolean }) {
  const [current, change] = useLayerOpen(open, defaultOpen, onOpenChange, baseEscape);
  return <DialogPrimitive.Root data-slot="dialog" open={current} onOpenChange={change} {...props} />;
}

function DialogTrigger(props: ComponentProps<typeof DialogPrimitive.Trigger>) {
  return <DialogPrimitive.Trigger data-slot="dialog-trigger" {...props} />;
}

function DialogClose(props: ComponentProps<typeof DialogPrimitive.Close>) {
  return <DialogPrimitive.Close data-slot="dialog-close" {...props} />;
}

function DialogOverlay({ className, ...props }: ComponentProps<typeof DialogPrimitive.Overlay>) {
  return <DialogPrimitive.Overlay data-slot="dialog-overlay" className={cn("fixed inset-0 z-50 bg-background opacity-(--opacity-secondary)", className)} {...props} />;
}

/**
 * The modal surface. Closing hands the keyboard back to whatever held it when
 * the dialog opened, a terminal included, or to `returnFocusTo` when its owner
 * names the place. `initialFocus="container"` focuses
 * the surface itself, so an irreversible choice has no default (design 6).
 */
function DialogContent({
  className,
  children,
  showCloseButton = false,
  initialFocus = "first",
  onOpenAutoFocus,
  onCloseAutoFocus,
  returnFocusTo,
  ...props
}: ComponentProps<typeof DialogPrimitive.Content> & { showCloseButton?: boolean; initialFocus?: "first" | "container"; returnFocusTo?: () => HTMLElement | null }) {
  const { t } = useInterfaceTranslation();
  const surface = useRef<HTMLDivElement>(null);
  const returnFocus = useReturnFocus(true, returnFocusTo ? { target: returnFocusTo, within: () => surface.current } : undefined);
  return (
    <DialogPrimitive.Portal>
      <DialogOverlay />
      <DialogPrimitive.Content
        ref={surface}
        data-slot="dialog-content"
        className={cn(
          "fixed left-1/2 top-1/2 z-50 flex max-h-[calc(100%-var(--spacing-xxl))] w-(--size-worktree-dialog) max-w-[calc(100%-var(--spacing-xxl))] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-lg border border-border bg-dialog text-body text-popover-foreground [--accent:var(--dialog-raised)] [--secondary:var(--dialog-raised)] shadow-lg outline-none",
          className,
        )}
        onOpenAutoFocus={(event) => {
          onOpenAutoFocus?.(event);
          if (event.defaultPrevented || initialFocus !== "container") return;
          event.preventDefault();
          surface.current?.focus();
        }}
        onCloseAutoFocus={(event) => {
          onCloseAutoFocus?.(event);
          if (!event.defaultPrevented) returnFocus(event);
        }}
        {...props}
      >
        {children}
        {showCloseButton ? (
          <DialogPrimitive.Close
            data-slot="dialog-close"
            aria-label={t("common.close")}
            className="absolute right-md top-md rounded-xs text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring [&_svg]:size-(--size-icon)"
          >
            <XIcon />
          </DialogPrimitive.Close>
        ) : null}
      </DialogPrimitive.Content>
    </DialogPrimitive.Portal>
  );
}

function DialogHeader({ className, ...props }: ComponentProps<"div">) {
  return <div data-slot="dialog-header" className={cn("flex flex-col gap-xs px-lg pt-lg", className)} {...props} />;
}

function DialogBody({ className, ...props }: ComponentProps<"div">) {
  return <div data-slot="dialog-body" className={cn("min-h-0 flex-1 overflow-auto px-lg py-md", className)} {...props} />;
}

function DialogFooter({ className, ...props }: ComponentProps<"div">) {
  return <div data-slot="dialog-footer" className={cn("flex flex-wrap items-center justify-end gap-sm px-lg pb-lg", className)} {...props} />;
}

function DialogTitle({ className, ...props }: ComponentProps<typeof DialogPrimitive.Title>) {
  return <DialogPrimitive.Title data-slot="dialog-title" className={cn("text-title font-semibold text-foreground", className)} {...props} />;
}

function DialogDescription({ className, ...props }: ComponentProps<typeof DialogPrimitive.Description>) {
  return <DialogPrimitive.Description data-slot="dialog-description" className={cn("text-body text-subtle-foreground", className)} {...props} />;
}

export { Dialog, DialogBody, DialogClose, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogOverlay, DialogTitle, DialogTrigger };
