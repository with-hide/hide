import { cva, type VariantProps } from "class-variance-authority";
import { ToggleGroup as ToggleGroupPrimitive } from "radix-ui";
import type { ComponentProps } from "react";
import { cn } from "../../lib/utils";

// A segmented choice: one value of a few, each shown at once (Layout, Working
// region, Provider, Theme). Radix gives it roving focus and arrow keys.
// `variant="pill"` draws the same choice as loose rounded chips that wrap, for
// a filter over a list whose options vary in number (Add a project's sources).
const toggleVariants = cva(
  "inline-flex items-center justify-center gap-xs whitespace-nowrap text-body font-medium text-subtle-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-(--opacity-disabled) [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-(--size-icon)",
  {
    variants: {
      variant: {
        segmented: "rounded-xs data-[state=on]:bg-secondary data-[state=on]:text-foreground",
        pill: "rounded-full border border-border hover:border-muted-foreground data-[state=on]:border-foreground data-[state=on]:bg-foreground data-[state=on]:text-background",
      },
      size: {
        default: "h-(--size-control-sm) px-sm",
        sm: "h-(--size-badge-height) px-xs text-caption",
        xs: "h-auto px-xs py-px text-micro",
      },
    },
    defaultVariants: { variant: "segmented", size: "default" },
  },
);

function ToggleGroup({ className, variant = "segmented", ...props }: ComponentProps<typeof ToggleGroupPrimitive.Root> & { variant?: "segmented" | "pill" }) {
  return (
    <ToggleGroupPrimitive.Root
      data-slot="toggle-group"
      className={cn("flex w-fit items-center", variant === "pill" ? "flex-wrap gap-xs" : "gap-xxs rounded-sm bg-card p-xxs", className)}
      {...props}
    />
  );
}

function ToggleGroupItem({ className, variant, size, ...props }: ComponentProps<typeof ToggleGroupPrimitive.Item> & VariantProps<typeof toggleVariants>) {
  return <ToggleGroupPrimitive.Item data-slot="toggle-group-item" className={cn(toggleVariants({ variant, size }), className)} {...props} />;
}

export { ToggleGroup, ToggleGroupItem, toggleVariants };
