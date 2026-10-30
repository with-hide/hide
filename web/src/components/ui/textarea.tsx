import type { ComponentProps } from "react";
import { cn } from "../../lib/utils";

/**
 * A multi-line text field, Input's border and focus ring by default. `bare`
 * drops the field's own frame for a composer whose container is the frame;
 * `mono` is for machine text, as on Input.
 */
function Textarea({ className, bare = false, mono = false, ...props }: ComponentProps<"textarea"> & { bare?: boolean; mono?: boolean }) {
  return (
    <textarea
      data-slot="textarea"
      className={cn(
        "block w-full min-w-0 resize-none text-body text-foreground outline-none transition-colors selection:bg-primary selection:text-primary-foreground placeholder:text-muted-foreground disabled:pointer-events-none disabled:opacity-(--opacity-disabled)",
        bare ? "bg-transparent" : "rounded-sm border border-input bg-background px-sm py-xs focus-visible:border-ring focus-visible:ring-1 focus-visible:ring-ring aria-invalid:border-destructive aria-invalid:ring-destructive",
        mono && "font-mono",
        className,
      )}
      {...props}
    />
  );
}

export { Textarea };
