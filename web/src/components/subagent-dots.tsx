import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { Hint } from "./ui/tooltip";

/** At most this many dots stand beside a title; the rest are counted after them (`+3`). */
export const SUBAGENT_DOTS_SHOWN = 3;

/**
 * The subagents an agent runs inside its own session, beside its title: one
 * small blue dot each, three at most and then `+N` (docs/UI_BEHAVIOR.md,
 * Subagents). Smaller and lighter than the status mark and a child's chip,
 * because a subagent has no pane or row of its own and nothing about it opens
 * anything. Nothing is drawn for none, and a count Hide cannot read reaches
 * here as none.
 *
 * `inRow` is a row whose own press, hover card and accessible name already
 * carry the words (`서브에이전트 N개 작업 중`), so the dots are only drawn there;
 * elsewhere they say them on hover and to a screen reader.
 */
export function SubagentDots({ count, inRow = false, className }: { count: number | undefined; inRow?: boolean; className?: string }) {
  const { t } = useInterfaceTranslation();
  if (!count) return null;
  const words = t("agents.subagents.working", { count });
  const dots = (
    <span
      {...(inRow ? { "aria-hidden": true } : { role: "img", "aria-label": words })}
      className={cn("inline-flex shrink-0 items-center gap-xxs opacity-(--opacity-secondary)", className)}
      data-subagent-dots={count}
    >
      {Array.from({ length: Math.min(count, SUBAGENT_DOTS_SHOWN) }, (_, index) => (
        <span key={index} className="size-(--size-subagent-dot) rounded-full bg-agent-working" />
      ))}
      {count > SUBAGENT_DOTS_SHOWN ? <span className="font-mono text-micro leading-none text-agent-working">+{count - SUBAGENT_DOTS_SHOWN}</span> : null}
    </span>
  );
  return inRow ? dots : <Hint label={words}>{dots}</Hint>;
}
