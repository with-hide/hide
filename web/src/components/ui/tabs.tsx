import { Tabs as TabsPrimitive } from "radix-ui";
import type { ComponentProps } from "react";
import { cn } from "../../lib/utils";

function Tabs({ className, ...props }: ComponentProps<typeof TabsPrimitive.Root>) {
  return <TabsPrimitive.Root data-slot="tabs" className={cn("flex flex-col gap-sm", className)} {...props} />;
}

// A segmented track is a faint wash of the text colour, so it reads on any
// surface under it, and its chosen segment is lifted on the raised surface.
// `variant="nav"` is a settings sheet's list of sections: no track, and the
// chosen section is filled as a row is. `variant="underline"` is a page's
// tabs over a divider: the chosen tab is its primary underline alone, never a
// fill, and a tab under the pointer shows a faint grey one.
function TabsList({ className, variant = "segmented", ...props }: ComponentProps<typeof TabsPrimitive.List> & { variant?: "segmented" | "nav" | "underline" }) {
  return <TabsPrimitive.List data-slot="tabs-list" data-variant={variant} className={cn("group/tabs inline-flex w-fit items-center gap-xxs rounded-sm p-xxs", variant === "segmented" && "bg-foreground/6", className)} {...props} />;
}

function TabsTrigger({ className, ...props }: ComponentProps<typeof TabsPrimitive.Trigger>) {
  return (
    <TabsPrimitive.Trigger
      data-slot="tabs-trigger"
      className={cn(
        "inline-flex h-(--size-control-sm) items-center justify-center gap-xs whitespace-nowrap rounded-xs px-sm text-body font-medium text-subtle-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-(--opacity-disabled) data-[state=active]:text-foreground group-data-[variant=nav]/tabs:data-[state=active]:bg-secondary group-data-[variant=segmented]/tabs:data-[state=active]:bg-raised group-data-[variant=segmented]/tabs:data-[state=active]:shadow-raised group-data-[variant=underline]/tabs:h-auto group-data-[variant=underline]/tabs:rounded-none group-data-[variant=underline]/tabs:border-b-2 group-data-[variant=underline]/tabs:border-transparent group-data-[variant=underline]/tabs:data-[state=inactive]:hover:border-foreground/20 group-data-[variant=underline]/tabs:data-[state=active]:border-primary [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-(--size-icon)",
        className,
      )}
      {...props}
    />
  );
}

function TabsContent({ className, ...props }: ComponentProps<typeof TabsPrimitive.Content>) {
  return <TabsPrimitive.Content data-slot="tabs-content" className={cn("flex-1 outline-none", className)} {...props} />;
}

export { Tabs, TabsList, TabsTrigger, TabsContent };
