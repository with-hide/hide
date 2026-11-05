import { LaptopIcon, ServerIcon } from "lucide-react";
import { cn } from "../lib/utils";
import { Badge } from "./ui/badge";

/** R1: a remote location is always the server glyph and Hide's device name; this Mac's own chip wears the laptop glyph. `remote` fills the chip in the other-device colour, as the workspace's header band does, for a map that must tell a node on another device at a glance. */
export function DeviceChip({ label, local = false, variant = "outline", className }: { label: string; local?: boolean; variant?: "outline" | "inline" | "remote"; className?: string }) {
  const Icon = local ? LaptopIcon : ServerIcon;
  return (
    <Badge variant="outline" className={cn("min-w-0 shrink gap-xxs", variant === "inline" && "h-auto rounded-none border-0 px-0 text-caption font-normal text-muted-foreground", variant === "remote" && "h-auto rounded-xs border-0 bg-device-remote px-xs text-caption font-medium text-primary-foreground", className)} data-device-chip={label}>
      <Icon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
      <span className="truncate">{label}</span>
    </Badge>
  );
}
