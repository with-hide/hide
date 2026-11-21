import { Command as CommandPrimitive } from "cmdk";
import { SearchIcon } from "lucide-react";
import type { ComponentProps, ReactNode } from "react";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "./dialog";

// The palette surface (shadcn Command over cmdk): a field, a list whose
// highlighted row the arrow keys move, and Enter to choose it.

function Command({ className, ...props }: ComponentProps<typeof CommandPrimitive>) {
  return (
    <CommandPrimitive
      data-slot="command"
      className={cn("flex h-full w-full flex-col overflow-hidden rounded-md bg-popover text-body text-popover-foreground", className)}
      {...props}
    />
  );
}

function CommandDialog({
  title,
  description,
  children,
  className,
  ...props
}: ComponentProps<typeof Dialog> & { title?: string; description?: string; className?: string }) {
  const { t } = useInterfaceTranslation();
  // The list draws on the dialog's own surface: the dialog lights a selected
  // row in `--dialog-hover`, a step under `bg-popover`, and on a popover's own
  // fill the selection would vanish.
  return (
    <Dialog {...props}>
      <DialogContent className={cn("top-(--size-settings-sheet-window-inset) w-(--size-search-sheet-w) translate-y-0 p-none [&_[data-slot=command]]:bg-transparent", className)}>
        <DialogTitle className="sr-only">{title ?? t("shell.commandPalette")}</DialogTitle>
        <DialogDescription className="sr-only">{description ?? t("shell.commandDescription")}</DialogDescription>
        {children}
      </DialogContent>
    </Dialog>
  );
}

/** `trailing` sits at the end of the field's row, such as a palette's Esc keycap. */
function CommandInput({ className, trailing, wrapperClassName, ...props }: ComponentProps<typeof CommandPrimitive.Input> & { trailing?: ReactNode; wrapperClassName?: string }) {
  return (
    <div data-slot="command-input-wrapper" className={cn("flex h-(--size-control-lg) items-center gap-sm border-b border-border px-md", wrapperClassName)}>
      <SearchIcon className="size-(--size-icon) shrink-0 text-muted-foreground" />
      <CommandPrimitive.Input
        data-slot="command-input"
        className={cn("flex h-full w-full bg-transparent text-body text-foreground outline-none placeholder:text-muted-foreground disabled:cursor-not-allowed disabled:opacity-(--opacity-disabled)", className)}
        {...props}
      />
      {trailing}
    </div>
  );
}

function CommandList({ className, ...props }: ComponentProps<typeof CommandPrimitive.List>) {
  return <CommandPrimitive.List data-slot="command-list" className={cn("max-h-(--size-search-sheet-h) scroll-py-xxs overflow-y-auto overflow-x-hidden p-xxs", className)} {...props} />;
}

function CommandEmpty({ className, ...props }: ComponentProps<typeof CommandPrimitive.Empty>) {
  return <CommandPrimitive.Empty data-slot="command-empty" className={cn("px-md py-sm text-caption text-muted-foreground", className)} {...props} />;
}

function CommandGroup({ className, ...props }: ComponentProps<typeof CommandPrimitive.Group>) {
  return (
    <CommandPrimitive.Group
      data-slot="command-group"
      className={cn("overflow-hidden text-foreground [&_[cmdk-group-heading]]:px-sm [&_[cmdk-group-heading]]:py-xs [&_[cmdk-group-heading]]:text-caption [&_[cmdk-group-heading]]:font-medium [&_[cmdk-group-heading]]:text-muted-foreground", className)}
      {...props}
    />
  );
}

function CommandSeparator({ className, ...props }: ComponentProps<typeof CommandPrimitive.Separator>) {
  return <CommandPrimitive.Separator data-slot="command-separator" className={cn("-mx-xxs h-(--size-hairline) bg-border", className)} {...props} />;
}

function CommandItem({ className, ...props }: ComponentProps<typeof CommandPrimitive.Item>) {
  return (
    <CommandPrimitive.Item
      data-slot="command-item"
      className={cn(
        "relative flex cursor-default select-none items-center gap-sm rounded-xs px-sm py-xs text-body outline-none data-[disabled=true]:pointer-events-none data-[selected=true]:bg-accent data-[selected=true]:text-accent-foreground data-[disabled=true]:text-muted-foreground [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-(--size-icon) [&_svg:not([class*='text-'])]:text-muted-foreground",
        className,
      )}
      {...props}
    />
  );
}

function CommandShortcut({ className, ...props }: ComponentProps<"span">) {
  return <span data-slot="command-shortcut" className={cn("ml-auto text-caption tracking-widest text-muted-foreground", className)} {...props} />;
}

export { Command, CommandDialog, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList, CommandSeparator, CommandShortcut };
