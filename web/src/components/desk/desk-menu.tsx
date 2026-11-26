import { useState, type ReactNode } from "react";
import { cn } from "../../lib/utils";
import { CommandList } from "../ui/command";

/**
 * The look every menu of Desk's composer shares (feedback 1 frame P 가): a
 * floating surface with a wide radius over the menus' own hairline ring and
 * shadow (`menu-styles.ts`, one step up inside a modal by `useFloatingSurface`),
 * group names small and bold in the muted color, rows with room around
 * them, and the highlighted row a rounded fill as wide as the panel.
 *
 * `DESK_MENU_PANEL` is the `/` and `@` panel's popover, as wide as the box
 * it stands on; `DESK_MENU_DROPDOWN` is a dropdown under the box (the
 * permission, agent, model and effort menus), whose rows are the dropdown's
 * own and take the same height and fill.
 */
export const DESK_MENU_PANEL = "w-(--radix-popover-trigger-width) rounded-xl p-none";

export const DESK_MENU_DROPDOWN =
  "rounded-xl p-xs [&_[role^=menuitem]]:rounded-md [&_[role^=menuitem]]:py-sm [&_[role^=menuitem][data-highlighted]]:bg-secondary! [&_[role^=menuitem][data-highlighted]]:text-foreground! [&_[role=menuitemradio][data-state=checked]]:bg-secondary [&_[role=menuitemradio]>span:first-child_.lucide-circle]:size-(--size-status-mark) [&_[role=menuitemradio]>span:first-child_.lucide-circle]:text-foreground";

/** A dropdown's group name in the panel's look: small, bold and muted. */
export const DESK_MENU_HEADING = "text-micro font-semibold text-muted-foreground";

/** The gap between the box and a menu over it, `--spacing-sm` as the number the popover takes. */
export const DESK_MENU_GAP = 8;

/** The panel's group names and rows inside `Command`. */
const ROWS =
  "px-xs pt-md pb-xs [&_[cmdk-group-heading]]:px-sm [&_[cmdk-group-heading]]:pt-none [&_[cmdk-group-heading]]:pb-xs [&_[cmdk-group-heading]]:text-micro [&_[cmdk-group-heading]]:font-semibold [&_[cmdk-group-heading]]:text-muted-foreground [&_[data-slot=command-item]]:rounded-md [&_[data-slot=command-item]]:px-sm [&_[data-slot=command-item]]:py-sm [&_[data-slot=command-item][data-selected=true]]:bg-secondary [&_[data-slot=command-item][data-selected=true]]:text-foreground";

/** Five two-line rows: a longer list scrolls inside, so the panel never covers the screen above the box. */
const FIVE_ROWS = "max-h-[calc((var(--text-body)*1.5*2+var(--spacing-sm)*2)*5+var(--spacing-md))]";

/**
 * The panel's list: its rows within five rows' height, and while more rows
 * wait below, the last one fades out.
 */
export function DeskMenuList({ className, children, ...data }: { className?: string; children: ReactNode } & Record<`data-${string}`, string>) {
  const [more, setMore] = useState(true);
  return (
    <CommandList
      className={cn(ROWS, FIVE_ROWS, more && "mask-b-from-80%", className)}
      onScroll={(event) => {
        const list = event.currentTarget;
        setMore(list.scrollHeight - list.scrollTop - list.clientHeight > 1);
      }}
      ref={(list) => {
        if (list) setMore(list.scrollHeight - list.scrollTop - list.clientHeight > 1);
      }}
      {...data}
    >
      {children}
    </CommandList>
  );
}

/** One line in the panel in place of its rows, under the group's name: nothing matches, or the list is on its way. */
export function DeskMenuLine({ heading, children, ...data }: { heading: string; children: ReactNode } & Record<`data-${string}`, string>) {
  return (
    <div className="flex flex-col px-xs pt-md pb-xs" {...data}>
      <span className="px-sm pb-xs text-micro font-semibold text-muted-foreground">{heading}</span>
      <span className="flex items-center gap-xs px-sm py-sm text-body text-muted-foreground">{children}</span>
    </div>
  );
}
