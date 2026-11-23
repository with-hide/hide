import { cva, type VariantProps } from "class-variance-authority";
import { Slot } from "radix-ui";
import type { ComponentProps } from "react";
import { cn } from "../../lib/utils";

const badgeVariants = cva(
  "inline-flex h-(--size-badge-height) w-fit shrink-0 items-center justify-center gap-xxs overflow-hidden whitespace-nowrap rounded-xs px-xs text-micro font-medium [&>svg]:pointer-events-none [&>svg]:size-(--size-icon-sm)",
  {
    variants: {
      variant: {
        default: "bg-primary text-primary-foreground",
        secondary: "bg-secondary text-subtle-foreground",
        destructive: "bg-destructive text-destructive-foreground",
        outline: "border border-border text-subtle-foreground",
        warning: "bg-warning/15 text-warning",
      },
    },
    defaultVariants: { variant: "secondary" },
  },
);

function Badge({ className, variant, asChild = false, ...props }: ComponentProps<"span"> & VariantProps<typeof badgeVariants> & { asChild?: boolean }) {
  const Comp = asChild ? Slot.Root : "span";
  return <Comp data-slot="badge" className={cn(badgeVariants({ variant }), className)} {...props} />;
}

export { Badge, badgeVariants };
