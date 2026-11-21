import { Columns2Icon } from "lucide-react";
import { cn } from "../lib/utils";

/**
 * Pair view's mark (PRD lane-desk-pair-view B22, D-17): ◧ in the pair view
 * accent, before a paired tab's name in the tab row and on its agent's
 * sidebar row, so the screen in pair view is told from the usual one in
 * more than one place. The words ride the tab's or row's own name.
 */
export function PairMark({ className, ...rest }: { className?: string } & Record<`data-${string}`, string>) {
  return <Columns2Icon aria-hidden="true" className={cn("size-(--size-icon-sm) shrink-0 text-pair-view", className)} {...rest} />;
}
