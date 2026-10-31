import { PinIcon } from "lucide-react";
import { cn } from "../lib/utils";
import { Button } from "./ui/button";
import { Hint } from "./ui/tooltip";

/**
 * A lane's pin, beside the `✕` of the column at the lane's edge (PRD
 * session-docs-and-assistant D-73, B32, B33): an empty pin while the lane
 * floats over what it sits beside, a filled one drawn pressed while it is
 * docked. `hint` says what a press does; `state` names it for the tests and
 * the native checks (`on`, `off`, or `narrow` for a pinned lane drawn over).
 */
export function LanePinButton({ pinned, label, hint, state, onPin }: { pinned: boolean; label: string; hint: string; state: "on" | "off" | "narrow"; onPin: () => void }) {
  return (
    <Hint label={hint}>
      <Button variant="ghost" size="icon-sm" className={pinned ? "text-foreground" : "text-muted-foreground"} aria-pressed={pinned} aria-label={label} aria-description={hint} data-lane-pin={state} onClick={onPin}>
        <PinIcon className={cn(pinned && "fill-current")} />
      </Button>
    </Hint>
  );
}
