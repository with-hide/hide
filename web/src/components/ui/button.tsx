import { cva, type VariantProps } from "class-variance-authority";
import { Slot } from "radix-ui";
import type { ComponentProps } from "react";
import { cn } from "../../lib/utils";

// shadcn Button on hide tokens; the gallery's Button section draws each
// variant and size under the same names (docs/DESIGN_GUIDE.md, Which part).
// `text` and `text-link` are actions that read as words in a row or a line of
// copy; with `size="inline"` they take the surrounding text's size and height. `row-action` is
// an icon action inside a row that lights on hover itself, so it lights one
// step further (secondary over the row's accent) and stays lit while its menu
// is open.
const buttonVariants = cva(
  "inline-flex shrink-0 items-center justify-center gap-xs whitespace-nowrap rounded-sm font-medium outline-none transition-colors focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-offset-1 focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-(--opacity-disabled) aria-invalid:ring-1 aria-invalid:ring-destructive [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-(--size-icon)",
  {
    variants: {
      variant: {
        default: "bg-primary text-primary-foreground hover:bg-primary/90",
        destructive: "bg-destructive text-destructive-foreground hover:bg-destructive/90",
        outline: "border border-border bg-background text-foreground hover:bg-accent hover:text-accent-foreground",
        secondary: "border border-border bg-secondary text-secondary-foreground hover:bg-accent",
        ghost: "text-subtle-foreground hover:bg-accent hover:text-accent-foreground",
        link: "text-primary underline-offset-4 hover:underline",
        text: "font-normal text-subtle-foreground hover:text-foreground",
        "text-link": "font-normal text-subtle-foreground underline-offset-4 hover:text-foreground hover:underline",
        "row-action": "text-muted-foreground hover:bg-secondary hover:text-foreground data-[state=open]:bg-secondary",
      },
      size: {
        default: "h-(--size-control) px-md text-body",
        sm: "h-(--size-control-sm) px-sm text-body",
        lg: "h-(--size-control-lg) px-lg text-body",
        icon: "size-(--size-control) text-body",
        "icon-sm": "size-(--size-control-sm) text-body",
        "icon-xs": "rounded-xs p-xxs text-body [&_svg:not([class*='size-'])]:size-(--size-icon-sm)",
        // A word action sits in a line of text: it keeps that text's size and
        // line height, and its ring hugs it.
        inline: "h-auto rounded-xs p-0 focus-visible:ring-offset-0",
      },
    },
    defaultVariants: { variant: "default", size: "default" },
  },
);

function Button({
  className,
  variant,
  size,
  asChild = false,
  type,
  ...props
}: ComponentProps<"button"> & VariantProps<typeof buttonVariants> & { asChild?: boolean }) {
  const Comp = asChild ? Slot.Root : "button";
  return (
    <Comp
      data-slot="button"
      type={asChild ? type : (type ?? "button")}
      className={cn(buttonVariants({ variant, size, className }))}
      {...props}
    />
  );
}

export { Button, buttonVariants };
