import { ChevronDownIcon, ChevronUpIcon, CircleHelpIcon } from "lucide-react";
import { useState } from "react";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import { Elapsed } from "../elapsed";
import type { AskLine } from "../open-asks";
import { Button } from "../ui/button";
import { Popover, PopoverAnchor, PopoverContent, PopoverTrigger } from "../ui/popover";
import { RowButton } from "../ui/row-button";

/**
 * The questions Hide found for the session, in one line above the input box
 * (Doop B4, question 나): the same list 내 차례 reads (`open_asks`), drawn in
 * the warning amber the sidebar's 내 차례 question rows use (`text-warning`),
 * not the pane's `질문 N` blue. It only shows: the answer is typed in the
 * box below, and an agent's own choice card stays in the conversation. The
 * line says the newest question and `외 N` for the rest, which open upward as
 * a list, newest first. Pressing a question goes to the answer it was asked in.
 */
export function ThreadAsks({ asks, onGo }: { asks: readonly AskLine[]; onGo: (ask: AskLine) => void }) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(false);
  const newest = [...asks].sort((a, b) => b.at_unix_ms - a.at_unix_ms);
  const first = newest[0];
  if (!first) return null;
  const more = newest.length - 1;
  const go = (ask: AskLine) => {
    setOpen(false);
    onGo(ask);
  };
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverAnchor asChild>
        <div className="flex min-w-0 items-center gap-xxs rounded-md border border-warning/50 bg-warning/10 px-xs text-caption" data-desk-thread-asks={asks.length}>
          <RowButton className="min-w-0 flex-1 gap-xs rounded-sm px-xxs py-xxs" aria-label={t("desk.asks.go", { question: first.text })} onClick={() => go(first)} data-desk-thread-ask={first.id}>
            <CircleHelpIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-warning" />
            <span className="shrink-0 font-semibold text-warning">{t("desk.ask.question")}</span>
            <span className="min-w-0 flex-1 truncate text-foreground" title={first.text}>{first.text}</span>
          </RowButton>
          {more > 0 ? (
            <PopoverTrigger asChild>
              <Button variant="text" size="inline" className="shrink-0 gap-xxs px-xxs text-micro text-subtle-foreground" aria-expanded={open} data-desk-thread-asks-more={more}>
                {t("desk.asks.more", { count: more })}
                {open ? <ChevronDownIcon aria-hidden="true" className="size-(--size-icon-sm)" /> : <ChevronUpIcon aria-hidden="true" className="size-(--size-icon-sm)" />}
              </Button>
            </PopoverTrigger>
          ) : null}
        </div>
      </PopoverAnchor>
      <PopoverContent side="top" align="start" className="flex w-(--radix-popover-trigger-width) min-w-0 flex-col gap-xxs border border-warning/50 p-xs" data-desk-thread-asks-list={newest.length}>
        <p className="px-xs text-micro text-muted-foreground">{t("desk.asks.head", { count: newest.length })}</p>
        <ol className="flex min-w-0 flex-col">
          {newest.map((ask, index) => (
            <li key={ask.id} className="min-w-0">
              <RowButton className={cn("gap-xs rounded-sm px-xs py-xxs text-caption", index === 0 && "bg-warning/10")} onClick={() => go(ask)} data-desk-thread-ask={ask.id}>
                <span className="w-(--size-icon-sm) shrink-0 text-micro font-semibold text-warning tabular-nums">{index + 1}</span>
                <span className="min-w-0 flex-1 truncate text-foreground" title={ask.text}>{ask.text}</span>
                <Elapsed since={ask.at_unix_ms} className="shrink-0 text-micro text-muted-foreground" />
              </RowButton>
            </li>
          ))}
        </ol>
      </PopoverContent>
    </Popover>
  );
}
