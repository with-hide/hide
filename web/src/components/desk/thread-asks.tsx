import { ChevronDownIcon, ChevronUpIcon, XIcon } from "lucide-react";
import { useState } from "react";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import { useElapsed } from "../elapsed";
import { useAwayAsks, type AskLine } from "../open-asks";
import { Button } from "../ui/button";
import { useEscapeLayer } from "../ui/layer";
import { RowButton, RowOverlayButton } from "../ui/row-button";
import { Hint } from "../ui/tooltip";

const NO_GONE: ReadonlySet<string> = new Set();

// One row is two lines of the question at most, with room above and below;
// the list is three rows tall, its hairlines included, and scrolls past that.
const LIST_HEIGHT = "max-h-[calc((2lh+var(--spacing-sm)*2)*3+var(--size-hairline)*2)]";

/**
 * The questions Hide found for the session, above the input box (Doop N3 가):
 * the same list 내 차례 reads (`open_asks`), oldest first, each in the agent's
 * own sentence Hide AI quoted for it (two lines at most; Hide AI's summary for
 * one kept before the sentence was), with when it was asked and `×`. The
 * question colour (`--open-ask`) is only on `질문` and each row's dot. It only
 * shows: the answer is typed in the box below, and an agent's own choice card
 * stays in the conversation. Pressing a question goes to the message it was
 * asked in and lights its sentence (`onGo`); `lit` is the one lit now, and
 * `gone` the ones whose message the conversation no longer holds, which say
 * so under their words and do nothing when pressed. `×` is 무시하기, which
 * takes the question out of 내 차례 and leaves it struck through with 되돌리기
 * while this session stays open (`useAwayAsks`). Several fold to `질문 N ·
 * first question ⌄` and unfold under a head of `질문 N` and `접기 ⌃`, one row
 * each in a list three rows tall that scrolls inside, so the input box below
 * never moves for it; Esc folds it again.
 */
export function ThreadAsks({ asks, onGo, onSet, lit = null, gone = NO_GONE }: {
  asks: readonly AskLine[];
  onGo: (ask: AskLine) => void;
  onSet: (id: string, status: "open" | "dismissed") => void;
  lit?: string | null;
  /** The questions whose message the conversation no longer holds (`askPlace`). */
  gone?: ReadonlySet<string>;
}) {
  const { t } = useInterfaceTranslation();
  const { lines, isAway, putAway, bringBack } = useAwayAsks(asks, onSet);
  const [listOpen, setListOpen] = useState(false);
  // While the list is unfolded, Esc folds it first, before it closes the panel the thread stands in.
  useEscapeLayer(listOpen && lines.length > 1, () => setListOpen(false));
  const first = lines[0];
  if (!first) return null;
  const open = lines.filter((ask) => !isAway(ask.id));
  const several = lines.length > 1;
  const row = (ask: AskLine, inList: boolean) => (
    <AskRow
      ask={ask}
      inList={inList}
      away={isAway(ask.id)}
      lit={lit === ask.id}
      gone={gone.has(ask.id)}
      onGo={() => onGo(ask)}
      onAway={() => putAway(ask)}
      onBack={() => bringBack(ask.id)}
    />
  );
  return (
    <div className="shrink-0 px-sm pt-xs">
      <div
        className="flex min-w-0 flex-col overflow-hidden rounded-lg border border-border bg-secondary text-body"
        data-desk-thread-asks={open.length}
        data-desk-thread-asks-open={several ? (listOpen ? "true" : "false") : undefined}
      >
        {!several ? (
          row(first, false)
        ) : listOpen ? (
          <>
            <div className="flex min-h-(--size-control-compact) min-w-0 items-center gap-xs pl-md pr-xs">
              <span className="min-w-0 flex-1 font-semibold text-open-ask" data-desk-thread-ask-lead="true">{t("desk.asks.count", { count: open.length })}</span>
              <FoldButton open onClick={() => setListOpen(false)} label={t("paneTurn.fold")} data-desk-thread-asks-fold="true" />
            </div>
            <ol className={cn("min-w-0 overflow-y-auto", LIST_HEIGHT)} data-desk-thread-asks-list={lines.length}>
              {lines.map((ask) => <li key={ask.id} className="min-w-0 border-t border-border">{row(ask, true)}</li>)}
            </ol>
          </>
        ) : (
          <RowButton className="gap-xs rounded-none px-md py-sm" aria-expanded={false} onClick={() => setListOpen(true)} data-desk-thread-asks-more={lines.length - 1}>
            <span className="shrink-0 font-semibold text-open-ask" data-desk-thread-ask-lead="true">{t("desk.asks.count", { count: open.length })} ·</span>
            <span className={cn("min-w-0 flex-1 truncate", isAway(first.id) && "text-muted-foreground line-through")}>{words(first)}</span>
            <ChevronDownIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
          </RowButton>
        )}
      </div>
    </div>
  );
}

/** What a question line says: the agent's own sentence, or Hide AI's summary for one kept without it. */
function words(ask: AskLine): string {
  return ask.quote || ask.text;
}

/**
 * One question: the whole row goes to where it was asked, with its words, when
 * it was asked and `×` or, once put away, struck through with 되돌리기. Alone
 * it is led by `질문`; in a list by its dot, the head above saying the rest.
 */
function AskRow({ ask, inList, away, lit, gone, onGo, onAway, onBack }: {
  ask: AskLine;
  inList: boolean;
  away: boolean;
  lit: boolean;
  gone: boolean;
  onGo: () => void;
  onAway: () => void;
  onBack: () => void;
}) {
  const { t } = useInterfaceTranslation();
  const said = words(ask);
  return (
    <div className={cn("relative flex min-w-0 items-start gap-xs py-sm pl-md pr-xs", lit && "bg-accent")} data-desk-thread-ask-row={ask.id} data-dismissed={away ? "true" : undefined} data-lit={lit ? "true" : undefined}>
      {away || gone ? null : <RowOverlayButton className="rounded-none" aria-label={t("desk.asks.go", { question: said })} onClick={onGo} data-desk-thread-ask={ask.id} />}
      <span className="pointer-events-none relative flex min-w-0 flex-1 items-start gap-xs">
        {inList ? (
          <span aria-hidden="true" className="flex h-[1lh] shrink-0 items-center"><span className={cn("size-(--size-tab-status-dot) rounded-full", away ? "bg-muted-foreground" : "bg-open-ask")} data-desk-thread-ask-dot="true" /></span>
        ) : (
          <span className={cn("shrink-0 font-semibold", away ? "text-muted-foreground" : "text-open-ask")} data-desk-thread-ask-lead="true">{t("desk.ask.question")}</span>
        )}
        <span className="flex min-w-0 flex-1 flex-col gap-xxs">
          <span className={cn("min-w-0", away ? "truncate text-muted-foreground line-through" : "line-clamp-2 text-foreground")} title={said} data-desk-thread-ask-words={ask.id}>{said}</span>
          {gone && !away ? <span className="text-micro text-muted-foreground" data-desk-thread-ask-gone={ask.id}>{t("desk.asks.gone")}</span> : null}
        </span>
      </span>
      <span className="relative flex shrink-0 items-center gap-xxs">
        {away ? (
          <>
            <span className="px-xxs text-caption text-muted-foreground">{t("desk.asks.dismissed")}</span>
            <Button variant="text" size="inline" className="px-xxs text-caption" onClick={onBack} data-desk-thread-ask-undo={ask.id}>{t("backlog.panel.undo")}</Button>
          </>
        ) : (
          <>
            <AskedAgo at={ask.at_unix_ms} />
            <Hint label={t("sessionWork.dismiss")}>
              <Button variant="row-action" size="icon-xs" aria-label={t("sessionWork.dismiss")} onClick={onAway} data-desk-thread-ask-dismiss={ask.id}>
                <XIcon aria-hidden="true" />
              </Button>
            </Hint>
          </>
        )}
      </span>
    </div>
  );
}

/** When a question was asked: `20초 전`. */
function AskedAgo({ at }: { at: number }) {
  const { t } = useInterfaceTranslation();
  const elapsed = useElapsed(at);
  return elapsed === null ? null : <span className="px-xxs text-caption text-muted-foreground" data-desk-thread-ask-ago="true">{t("desk.asks.ago", { time: elapsed })}</span>;
}

/** A word that folds the list, with its chevron. */
function FoldButton({ open, label, onClick, ...rest }: { open: boolean; label: string; onClick: () => void } & Record<`data-${string}`, string | number | undefined>) {
  return (
    <Button variant="text" size="inline" className="shrink-0 gap-xxs px-xxs text-caption text-subtle-foreground" aria-expanded={open} onClick={onClick} {...rest}>
      {label}
      {open ? <ChevronUpIcon aria-hidden="true" className="size-(--size-icon-sm)" /> : <ChevronDownIcon aria-hidden="true" className="size-(--size-icon-sm)" />}
    </Button>
  );
}
