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

/** 다시 볼 것 on a root agent (PRD revisit-group D-08, B10): `star` offers the ★, `done` offers 다시 볼 것에서 빼기. */
export type RevisitOffer = "hidden" | "star" | "done";

/**
 * What an agent's menu offers about 다시 볼 것, the same in its pane's menu and
 * its sidebar row's: the ★ is offered on a root the operator owns, and Hide AI,
 * a delegated agent and a child are offered none (the core itself refuses only a child's).
 */
export function revisitOffer(agent: Pick<AgentRow, "hide_ai" | "delegated" | "lineage_parent_pane_id" | "revisit">): RevisitOffer {
  if (agent.hide_ai || agent.delegated || agent.lineage_parent_pane_id) return "hidden";
  return starred(agent) ? "done" : "star";
}

/** The menu item for an offer that is not `hidden`: 다시 볼 것으로 표시, or 다시 볼 것에서 빼기 on a task that wears the ★. */
export function revisitMenuItem(offer: Exclude<RevisitOffer, "hidden">, t: (key: "taskLens.menu.revisit" | "taskLens.menu.revisitDone") => string) {
  return { id: "revisit" as const, icon: offer === "done" ? ("unstar" as const) : ("star" as const), label: t(offer === "done" ? "taskLens.menu.revisitDone" : "taskLens.menu.revisit"), unavailable: null };
}
