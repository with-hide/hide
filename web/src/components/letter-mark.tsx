import { MailIcon } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "../lib/utils";
import { useInterfaceTranslation } from "../i18n/client";
import { Hint } from "./ui/tooltip";

/**
 * Letters wait on a pane because a draft may stand in its composer, and the
 * pane has rested long enough for the core to show them (`letters_waiting`,
 * docs/delivery.md). In the pane header the pill after the title is the
 * button: the envelope, the count from two letters, and the action word,
 * which a narrow pane drops (docs/UI_BEHAVIOR.md, Agent panes). The popover
 * fill and the border edge set it apart from a focused header's secondary
 * fill and an unfocused header's card fill in every theme. It sits right
 * above the composer, so the operator sees whether it is empty before
 * pressing; one press is one event, and the core rings as its doorbell would.
 */
export function PaneLetterButton({ paneId, letters, onDeliver }: { paneId: string; letters: number; onDeliver: (paneId: string) => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <Hint label={t("panes.letters.deliverLabel", { count: letters })}>
      <button
        type="button"
        className="flex h-(--size-sidebar-line) shrink-0 items-center gap-xs rounded-xs border border-border bg-popover px-xs text-caption outline-none hover:border-muted-foreground focus-visible:ring-1 focus-visible:ring-ring"
        data-pane-letters={letters}
        onClick={() => onDeliver(paneId)}
      >
        <MailIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-subtle-foreground" />
        {letters > 1 ? <span aria-hidden="true" className="font-mono text-subtle-foreground">{letters}</span> : null}
        <span aria-hidden="true" className="hidden whitespace-nowrap text-foreground pane-wide:inline">
          {t("panes.letters.deliver")}
        </span>
      </button>
    </Hint>
  );
}

/**
 * The sidebar row's quiet mark for the same fact: an envelope in the muted
 * colour beside the time, with no number, quieter than any status mark. The
 * row's own button says it to assistive technology, and clicking the row goes
 * to the pane as always.
 */
export function RowLetterMark() {
  return <MailIcon aria-hidden="true" className="pointer-events-none size-(--size-icon-sm) shrink-0 text-muted-foreground" data-agent-letters="true" />;
}

/**
 * A letter's envelope and who it is between, at the head of the letter's
 * line (PRD session-docs-and-assistant D-72): the session record's request
 * from a sender. `who` is the name, or a mention chip where the line names a
 * session; `label` is what assistive technology reads for the pair.
 */
export function LetterTag({ who, label, className }: { who: ReactNode; label: string; className?: string }) {
  return (
    <span className={cn("inline-flex shrink-0 items-center gap-xxs text-micro", className)} aria-label={label} data-letter-tag="true">
      <MailIcon aria-hidden="true" className="size-(--size-icon-sm) self-center" />
      {who}
    </span>
  );
}
