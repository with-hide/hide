import { cva, type VariantProps } from "class-variance-authority";
import { ToggleGroup as ToggleGroupPrimitive } from "radix-ui";
import type { ComponentProps } from "react";
import { cn } from "../../lib/utils";

// A segmented choice: one value of a few, each shown at once (Layout, Working
// region, Provider, Theme). Radix gives it roving focus and arrow keys.
const toggleVariants = cva(
  "inline-flex items-center justify-center gap-xs whitespace-nowrap rounded-xs text-body font-medium text-subtle-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-(--opacity-disabled) data-[state=on]:bg-secondary data-[state=on]:text-foreground [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-(--size-icon)",
  {
    variants: {
      size: {
        default: "h-(--size-control-sm) px-sm",
        sm: "h-(--size-badge-height) px-xs text-caption",
        xs: "h-auto px-xs py-px text-micro",
      },
    },
    defaultVariants: { size: "default" },
  },
);

function ToggleGroup({ className, ...props }: ComponentProps<typeof ToggleGroupPrimitive.Root>) {
  return (
    <ToggleGroupPrimitive.Root
      data-slot="toggle-group"
      className={cn("flex w-fit items-center gap-xxs rounded-sm bg-card p-xxs", className)}
      {...props}
    />
  );
}

function ToggleGroupItem({ className, size, ...props }: ComponentProps<typeof ToggleGroupPrimitive.Item> & VariantProps<typeof toggleVariants>) {
  return <ToggleGroupPrimitive.Item data-slot="toggle-group-item" className={cn(toggleVariants({ size }), className)} {...props} />;
}

export { ToggleGroup, ToggleGroupItem, toggleVariants };
