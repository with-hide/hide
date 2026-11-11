import { StarIcon } from "lucide-react";
import type { HTMLAttributes } from "react";
import { useInterfaceTranslation } from "../i18n/client";
import type { AgentRow } from "../snapshot";
import { cn } from "../lib/utils";
import { Hint } from "./ui/tooltip";

/**
 * The operator's ★ (PRD revisit-group D-03, D-04): in the status mark's slot on
 * a starred task waiting in 다시 볼 것, and small after the title wherever the
 * task stands elsewhere. The row's own name reads the words, so the mark is
 * not announced.
 */
export function RevisitStar({ slot = false, className }: { slot?: boolean; className?: string }) {
  const { t } = useInterfaceTranslation();
  return (
    <Hint label={t("taskLens.revisit.starred")}>
      <RevisitStarShape slot={slot} className={className} />
    </Hint>
  );
}

/** The ★ alone, without the pointer's hint: the phone, which has no pointer, draws it where the desktop does. */
export function RevisitStarShape({ slot = false, className, ...rest }: { slot?: boolean; className?: string } & Omit<HTMLAttributes<HTMLSpanElement>, "slot">) {
  return (
    <span aria-hidden="true" data-revisit-star={slot ? "mark" : "title"} {...rest}
      className={cn("inline-flex shrink-0 items-center justify-center text-warning", slot ? "size-(--size-agent-mark)" : "", className)}>
      <StarIcon className={cn("fill-current", slot ? "size-full" : "size-(--size-icon-sm)")} />
    </span>
  );
}

/** Whether the row carries the operator's ★. */
export function starred(agent: Pick<AgentRow, "revisit">): boolean {
  return agent.revisit?.starred === true;
}
