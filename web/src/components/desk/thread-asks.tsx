import { ArrowUpIcon, ChevronDownIcon, ChevronUpIcon, XIcon } from "lucide-react";
import { useState, type ReactNode } from "react";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import { Elapsed } from "../elapsed";
import { useAwayAsks, type AskLine } from "../open-asks";
import { Button } from "../ui/button";
import { RowButton } from "../ui/row-button";
import { Hint } from "../ui/tooltip";

/**
 * The questions Hide found for the session, above the input box (Doop B4
 * question 나, N 2): the same list 내 차례 reads (`open_asks`), in the warning
 * amber the sidebar's 내 차례 question rows wear, oldest first. It only
 * shows: the answer is typed in the box below, and an agent's own choice card
 * stays in the conversation. Pressing a question goes to the answer it was
 * asked in and lights it; `본문 보기` unfolds that answer's words here; `×` is
 * 무시하기, which takes the question out of 내 차례 and leaves it struck
 * through with 되돌리기 while this session stays open (`useAwayAsks`). Several
 * fold to the first and `외 N`, and unfold one per line in a list three lines
 * tall that scrolls inside, so the input box below never moves for it.
 */
export function ThreadAsks({ asks, onGo, onSet, body }: {
  asks: readonly AskLine[];
  onGo: (ask: AskLine) => void;
  onSet: (id: string, status: "open" | "dismissed") => void;
  /** The words of the answer a question was asked in, null while that answer is not read yet. */
  body: (ask: AskLine) => ReactNode | null;
}) {
  const { t } = useInterfaceTranslation();
  const { lines, isAway, putAway, bringBack } = useAwayAsks(asks, onSet);
  const [listOpen, setListOpen] = useState(false);
  const [bodyOf, setBodyOf] = useState<string | null>(null);
  const first = lines[0];
  if (!first) return null;
  const open = lines.filter((ask) => !isAway(ask.id));
  const toggleBody = (ask: AskLine) => setBodyOf((now) => (now === ask.id ? null : ask.id));
  const row = (ask: AskLine, head: boolean) => (
    <AskRow
      ask={ask}
      head={head}
      away={isAway(ask.id)}
      bodyOpen={bodyOf === ask.id}
      onGo={() => onGo(ask)}
      onBody={() => toggleBody(ask)}
      onAway={() => {
        if (bodyOf === ask.id) setBodyOf(null);
        putAway(ask);
      }}
      onBack={() => bringBack(ask.id)}
    />
  );
  const several = lines.length > 1;
  // The unfolded message stands under its line, or under the list, which keeps its three lines.
  const unfolded = lines.find((ask) => ask.id === bodyOf && !isAway(ask.id)) ?? null;
  const words = unfolded && (!several || listOpen) ? <AskWords ask={unfolded} body={body(unfolded)} onGo={() => onGo(unfolded)} /> : null;
  return (
    <div className="shrink-0 px-sm pt-xs">
      <div
        className={cn("flex min-w-0 flex-col rounded-md border px-xs text-caption", open.length ? "border-warning/50 bg-warning/10" : "border-border")}
        data-desk-thread-asks={open.length}
        data-desk-thread-asks-open={several ? (listOpen ? "true" : "false") : undefined}
      >
        {!several ? (
          <>
            {row(first, true)}
            {words}
          </>
        ) : listOpen ? (
          <>
            <div className="flex min-w-0 items-center gap-xs py-xxs">
              <span className="flex-1 px-xxs font-semibold text-warning">{t("desk.asks.count", { count: open.length })}</span>
              <FoldButton open onClick={() => setListOpen(false)} label={t("paneTurn.fold")} data-desk-thread-asks-fold="true" />
            </div>
            {/* Three lines tall at most; a longer list scrolls inside. */}
            <ol className="-mx-xs max-h-[calc(var(--size-control-compact)*3)] min-w-0 overflow-y-auto border-t border-warning/30 px-xs" data-desk-thread-asks-list={lines.length}>
              {lines.map((ask) => <li key={ask.id} className="min-w-0 border-b border-warning/20 last:border-b-0">{row(ask, false)}</li>)}
            </ol>
            {words ? <div className="border-t border-warning/30 pt-xs">{words}</div> : null}
          </>
        ) : (
          <div className="flex min-w-0 items-center gap-xxs">
            <QuestionButton ask={first} away={isAway(first.id)} onGo={() => onGo(first)} />
            <FoldButton open={false} onClick={() => setListOpen(true)} label={t("desk.asks.more", { count: lines.length - 1 })} data-desk-thread-asks-more={lines.length - 1} />
          </div>
        )}
      </div>
    </div>
  );
}

/** One question: its words that go to its answer, its time in a list, 본문 보기, and × or, once put away, 되돌리기. */
function AskRow({ ask, head, away, bodyOpen, onGo, onBody, onAway, onBack }: {
  ask: AskLine;
  /** The line stands alone, led by `질문`; in a list the head above says it. */
  head: boolean;
  away: boolean;
  bodyOpen: boolean;
  onGo: () => void;
  onBody: () => void;
  onAway: () => void;
  onBack: () => void;
}) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex min-w-0 flex-col" data-desk-thread-ask-row={ask.id} data-dismissed={away ? "true" : undefined}>
      <div className="flex min-h-(--size-control-compact) min-w-0 items-center gap-xxs">
        <QuestionButton ask={ask} away={away} lead={head} onGo={onGo} />
        {!head && !away ? <Elapsed since={ask.at_unix_ms} className="shrink-0 text-micro text-muted-foreground" /> : null}
        {away ? (
          <>
            <span className="shrink-0 px-xxs text-micro text-muted-foreground">{t("desk.asks.dismissed")}</span>
            <Button variant="text" size="inline" className="shrink-0 px-xxs text-micro" onClick={onBack} data-desk-thread-ask-undo={ask.id}>{t("backlog.panel.undo")}</Button>
          </>
        ) : (
          <>
            <FoldButton open={bodyOpen} onClick={onBody} label={bodyOpen ? t("paneTurn.fold") : t("desk.asks.body")} data-desk-thread-ask-body={ask.id} />
            <Hint label={t("sessionWork.dismiss")}>
              <Button variant="row-action" size="icon-xs" aria-label={t("sessionWork.dismiss")} onClick={onAway} data-desk-thread-ask-dismiss={ask.id}>
                <XIcon aria-hidden="true" />
              </Button>
            </Hint>
          </>
        )}
      </div>
    </div>
  );
}

/** The words of the answer a question was asked in, in a box that scrolls past its height, with the way to it in the conversation. */
function AskWords({ ask, body, onGo }: { ask: AskLine; body: ReactNode | null; onGo: () => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="mb-xs flex min-w-0 flex-col gap-xxs rounded-sm border border-warning/30 px-sm py-xs" data-desk-thread-ask-words={ask.id}>
      <div className="max-h-[calc(var(--size-control-compact)*6)] min-w-0 overflow-y-auto">
        {body ?? <p className="text-muted-foreground">{t("desk.asks.unread")}</p>}
      </div>
      <Button variant="text" size="inline" className="gap-xxs self-start text-micro" onClick={onGo} data-desk-thread-ask-go={ask.id}>
        {t("desk.asks.inThread")}
        <ArrowUpIcon aria-hidden="true" className="size-(--size-icon-sm)" />
      </Button>
    </div>
  );
}

/** A question's words: pressing them goes to the answer it was asked in; one put away is struck through. */
function QuestionButton({ ask, away, lead = true, onGo }: { ask: AskLine; away: boolean; lead?: boolean; onGo: () => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <RowButton className="min-w-0 flex-1 gap-xs rounded-sm px-xxs py-xxs" aria-label={t("desk.asks.go", { question: ask.text })} onClick={onGo} data-desk-thread-ask={ask.id}>
      {lead ? <span className={cn("shrink-0 font-semibold", away ? "text-muted-foreground" : "text-warning")}>{t("desk.ask.question")}</span> : null}
      <span className={cn("min-w-0 flex-1 truncate", away ? "text-muted-foreground line-through" : "text-foreground")} title={ask.text}>{ask.text}</span>
    </RowButton>
  );
}

/** A word that folds or unfolds, with its chevron. */
function FoldButton({ open, label, onClick, ...rest }: { open: boolean; label: string; onClick: () => void } & Record<`data-${string}`, string | number | undefined>) {
  return (
    <Button variant="text" size="inline" className="shrink-0 gap-xxs px-xxs text-micro text-subtle-foreground" aria-expanded={open} onClick={onClick} {...rest}>
      {label}
      {open ? <ChevronUpIcon aria-hidden="true" className="size-(--size-icon-sm)" /> : <ChevronDownIcon aria-hidden="true" className="size-(--size-icon-sm)" />}
    </Button>
  );
}
