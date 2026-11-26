import { ArrowUpIcon, ChevronDownIcon, ChevronUpIcon, XIcon } from "lucide-react";
import { useState, type ReactNode } from "react";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import { useElapsed } from "../elapsed";
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
 * asked in and lights it; `본문 보기` unfolds that answer's words right under
 * its line; `×` is 무시하기, which takes the question out of 내 차례 and leaves
 * it struck through with 되돌리기 while this session stays open
 * (`useAwayAsks`). Several fold to the first and `외 N`, and unfold under a
 * head of `질문 N` and the first question, one per line in a list three lines
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
  const row = (ask: AskLine, inList: boolean) => {
    const away = isAway(ask.id);
    const bodyOpen = bodyOf === ask.id && !away;
    return (
      <AskRow
        ask={ask}
        lead={inList ? null : t("desk.ask.question")}
        time={inList}
        away={away}
        bodyOpen={bodyOpen}
        onGo={() => onGo(ask)}
        onBody={() => setBodyOf((now) => (now === ask.id ? null : ask.id))}
        onAway={() => {
          if (bodyOf === ask.id) setBodyOf(null);
          putAway(ask);
        }}
        onBack={() => bringBack(ask.id)}
        words={bodyOpen ? <AskWords ask={ask} body={body(ask)} onGo={() => onGo(ask)} /> : null}
      />
    );
  };
  const several = lines.length > 1;
  const bodyInList = several && listOpen && lines.some((ask) => ask.id === bodyOf && !isAway(ask.id));
  return (
    <div className="shrink-0 px-sm pt-xs">
      <div
        className={cn("flex min-w-0 flex-col rounded-md border px-xs text-caption", open.length ? "border-warning/50 bg-warning/10" : "border-border")}
        data-desk-thread-asks={open.length}
        data-desk-thread-asks-open={several ? (listOpen ? "true" : "false") : undefined}
      >
        {!several ? (
          row(first, false)
        ) : listOpen ? (
          <>
            <div className="flex min-h-(--size-control-compact) min-w-0 items-center gap-xxs">
              <QuestionButton ask={first} away={isAway(first.id)} lead={t("desk.asks.count", { count: open.length })} onGo={() => onGo(first)} />
              <FoldButton open onClick={() => setListOpen(false)} label={t("paneTurn.fold")} data-desk-thread-asks-fold="true" />
            </div>
            {/* Three lines tall at most, their hairlines included; a longer list, or one holding an unfolded message, scrolls inside. */}
            <ol
              className={cn(
                "-mx-xs min-w-0 overflow-y-auto border-t border-warning/30 px-xs",
                bodyInList ? "max-h-[calc(var(--size-control-compact)*9+var(--size-hairline)*3)]" : "max-h-[calc(var(--size-control-compact)*3+var(--size-hairline)*3)]",
              )}
              data-desk-thread-asks-list={lines.length}
            >
              {lines.map((ask) => <li key={ask.id} className="min-w-0 border-b border-warning/20 last:border-b-0">{row(ask, true)}</li>)}
            </ol>
          </>
        ) : (
          <div className="flex min-h-(--size-control-compact) min-w-0 items-center gap-xxs">
            <QuestionButton ask={first} away={isAway(first.id)} lead={t("desk.ask.question")} onGo={() => onGo(first)} />
            <FoldButton open={false} onClick={() => setListOpen(true)} label={t("desk.asks.more", { count: lines.length - 1 })} data-desk-thread-asks-more={lines.length - 1} />
          </div>
        )}
      </div>
    </div>
  );
}

/** One question: its words that go to its answer, when it was asked in a list, 본문 보기 and × or, once put away, 되돌리기, and its unfolded message right under it. */
function AskRow({ ask, lead, time, away, bodyOpen, words, onGo, onBody, onAway, onBack }: {
  ask: AskLine;
  /** The line stands alone, led by `질문`; in a list the head above says it. */
  lead: string | null;
  time: boolean;
  away: boolean;
  bodyOpen: boolean;
  words: ReactNode;
  onGo: () => void;
  onBody: () => void;
  onAway: () => void;
  onBack: () => void;
}) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex min-w-0 flex-col" data-desk-thread-ask-row={ask.id} data-dismissed={away ? "true" : undefined}>
      <div className="flex min-h-(--size-control-compact) min-w-0 items-center gap-xxs">
        <QuestionButton ask={ask} away={away} lead={lead} onGo={onGo} />
        {time && !away ? <AskedAgo at={ask.at_unix_ms} /> : null}
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
      {words}
    </div>
  );
}

/** When a question in the list was asked: `12분 전`. */
function AskedAgo({ at }: { at: number }) {
  const { t } = useInterfaceTranslation();
  const elapsed = useElapsed(at);
  return elapsed === null ? null : <span className="shrink-0 text-micro text-muted-foreground" data-desk-thread-ask-ago="true">{t("desk.asks.ago", { time: elapsed })}</span>;
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

/** A question's words after its lead (`질문`, or `질문 N` heading a list): pressing them goes to the answer it was asked in; one put away is struck through. */
function QuestionButton({ ask, away, lead, onGo }: { ask: AskLine; away: boolean; lead: string | null; onGo: () => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <RowButton className="min-w-0 flex-1 gap-xs rounded-sm px-xxs py-xxs" aria-label={t("desk.asks.go", { question: ask.text })} onClick={onGo} data-desk-thread-ask={ask.id}>
      {lead ? <span className={cn("shrink-0 font-semibold", away ? "text-muted-foreground" : "text-warning")} data-desk-thread-ask-lead="true">{lead}</span> : null}
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
