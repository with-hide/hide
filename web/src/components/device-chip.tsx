import { LaptopIcon, ServerIcon } from "lucide-react";
import { cn } from "../lib/utils";
import { Badge } from "./ui/badge";

/** R1: a remote location is always the server glyph and Hide's device name; this Mac's own chip wears the laptop glyph. */
export function DeviceChip({ label, local = false, variant = "outline", className }: { label: string; local?: boolean; variant?: "outline" | "inline"; className?: string }) {
  const Icon = local ? LaptopIcon : ServerIcon;
  return (
    <Badge variant="outline" className={cn("min-w-0 shrink gap-xxs", variant === "inline" && "h-auto rounded-none border-0 px-0 text-caption font-normal text-muted-foreground", className)} data-device-chip={label}>
      <Icon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
      <span className="truncate">{label}</span>
    </Badge>
  );
}
