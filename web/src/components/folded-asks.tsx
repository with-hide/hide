import { ChevronRightIcon } from "lucide-react";
import { useState } from "react";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { Button } from "./ui/button";

/** A question Hide AI folded: the record's id, its words and why it was folded. */
export type FoldedAsk = { id: string; text: string; reason?: string };

/**
 * 정리됨 (status-model.md, A question still waiting): the questions Hide AI
 * read as settled in a turn no answer came in, kept under one quiet line
 * rather than removed. Closed at first; opened, each line says why, and
 * 다시 열기 puts it back among the open questions as the operator's to answer.
 * Nothing is drawn while none is folded.
 */
export function FoldedAskList({ asks, onReopen }: { asks: readonly FoldedAsk[]; onReopen: (id: string) => void }) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(false);
  if (!asks.length) return null;
  return (
    <section className="mt-sm min-w-0" data-record-folded-asks={asks.length}>
      <Button variant="text" size="inline" className="gap-xxs text-caption" aria-expanded={open} onClick={() => setOpen((was) => !was)} data-record-folded-toggle="true">
        <ChevronRightIcon aria-hidden="true" className={cn("size-(--size-icon-sm)", open && "rotate-90")} />
        {t("record.folded", { count: asks.length })}
      </Button>
      {open ? (
        <ul className="mt-xxs min-w-0">
          {asks.map((ask) => (
            <li key={ask.id} className="flex min-w-0 items-start gap-xs border-b border-border py-xs last:border-b-0" data-record-folded-ask={ask.id}>
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="whitespace-normal text-subtle-foreground [overflow-wrap:anywhere]" title={ask.text}>{ask.text}</span>
                {ask.reason ? <span className="whitespace-normal text-micro text-muted-foreground [overflow-wrap:anywhere]" data-record-folded-reason="true">{ask.reason}</span> : null}
              </span>
              <Button size="sm" variant="ghost" className="h-auto shrink-0 px-xs py-0 text-micro" onClick={() => onReopen(ask.id)} data-record-folded-reopen={ask.id}>{t("record.foldedReopen")}</Button>
            </li>
          ))}
        </ul>
      ) : null}
    </section>
  );
}
