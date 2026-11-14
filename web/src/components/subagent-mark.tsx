import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { Hint } from "./ui/tooltip";

/**
 * The orbit icon: a faint ring with one filled dot on it, the work map's
 * session circle and its orbiting subagents drawn small. Its colour is the
 * text colour around it.
 */
function OrbitIcon({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 13 13" aria-hidden="true" className={cn("size-(--size-icon-sm) shrink-0", className)}>
      <circle cx="6.5" cy="6.5" r="4.2" fill="none" stroke="currentColor" strokeWidth="1.2" className="opacity-(--opacity-dimmed)" />
      <circle cx="9.5" cy="3.5" r="1.9" fill="currentColor" />
    </svg>
  );
}

/**
 * The subagents an agent runs inside its own session, right after its title:
 * the orbit icon and how many run, in the working colour (docs/UI_BEHAVIOR.md,
 * Subagents). One mark of one width whatever the count, so it reads as
 * neither an ellipsis, a "more" menu nor the unread dot, and a narrow row
 * shortens its title rather than losing the count. Nothing is drawn for
 * none, and a count Hide cannot read reaches here as none.
 *
 * `inRow` is a row whose own press, hover card and accessible name already
 * carry the words (`서브에이전트 N개 작업 중`), so the mark is only drawn there;
 * elsewhere it says them on hover and to a screen reader.
 */
export function SubagentMark({ count, inRow = false, className }: { count: number | undefined; inRow?: boolean; className?: string }) {
  const { t } = useInterfaceTranslation();
  if (!count) return null;
  const words = t("agents.subagents.working", { count });
  const mark = (
    <span
      {...(inRow ? { "aria-hidden": true } : { role: "img", "aria-label": words })}
      className={cn("inline-flex shrink-0 items-center gap-xxs font-mono text-caption tabular-nums text-agent-working", className)}
      data-subagent-mark={count}
    >
      <OrbitIcon />
      {count}
    </span>
  );
  return inRow ? mark : <Hint label={words}>{mark}</Hint>;
}
