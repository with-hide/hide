// The surface and item classes the Dropdown Menu, Context Menu and Select
// share, so a menu reads the same wherever it opens.
export const menuContent =
  "z-50 min-w-(--size-settings-control-w) overflow-y-auto overflow-x-hidden rounded-md border border-border bg-popover p-xxs text-body text-popover-foreground shadow-overlay outline-none";

export const menuItem =
  "relative flex cursor-default select-none items-center gap-sm rounded-xs px-sm py-xs text-body outline-none data-[disabled]:pointer-events-none data-[highlighted]:bg-accent data-[highlighted]:text-accent-foreground data-[disabled]:text-muted-foreground data-[variant=destructive]:text-destructive data-[inset]:pl-xl [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-(--size-icon) [&_svg:not([class*='text-'])]:text-muted-foreground";

export const menuLabel = "px-sm py-xs text-caption font-medium text-muted-foreground data-[inset]:pl-xl";

export const menuSeparator = "-mx-xxs my-xxs h-(--size-hairline) bg-border";

export const menuShortcut = "ml-auto text-caption tracking-widest text-muted-foreground";
