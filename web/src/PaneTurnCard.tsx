import { useRef, useState, type FocusEvent, type KeyboardEvent, type ReactNode } from "react";
import type { TFunction } from "i18next";
import type { Actions } from "./actions";
import { askWhat } from "./components/agent-tree";
import { Button } from "./components/ui/button";
import { Popover, PopoverAnchor, PopoverContent } from "./components/ui/popover";
import { statusText } from "./agentStatus";
import { formatDateTime } from "./i18n/format";
import { useInterfaceTranslation } from "./i18n/client";
import { requireInterfaceLanguage } from "./i18n/locale";
import { cn } from "./lib/utils";
import { renderMarkdown } from "./mobile/markdown";
import { agentsByPane, allAgents } from "./paneAgents";
import { sessionOfAgent } from "./sessionWork";
import { type AgentRow, type AskedTurn, type LabelEnd, type LetterLine, type LetterMark, type PaneHeader, type PaneTurnRequest, type RequestSender, type TurnAnswer, type TurnAnswerRead } from "./snapshot";
import { useShellStore } from "./store";
import { cardLines, letterCount, lettersBefore } from "./turnCard";

/** The handlers a turn card's trigger wears. */
export type TurnCardTrigger = {
  open: boolean;
  pinned: boolean;
  onFocus: () => void;
  onBlur: (event: FocusEvent) => void;
  /** Moves the keyboard into an open card on Tab. */
  onKeyDown: (event: KeyboardEvent) => void;
  /** Pins or unpins the card. */
  onToggle: () => void;
};

/**
 * The pane's turn card (PRD pane-last-request D-05, B6-B9): what the
 * operator asked, what the agent is doing about it and the turns before,
 * under the request on the pane's second row, its only trigger. A press or
 * the keyboard's focus on the request opens it, never the pointer passing
 * over, which kept raising it in the way of the terminal; a second press or
 * Esc closes it. Opening never takes the keyboard from the terminal.
 */
export function PaneTurnCard({ paneId, actions, trigger }: { paneId: string; actions: Actions; trigger: (state: TurnCardTrigger) => ReactNode }) {
  const [focused, setFocused] = useState(false);
  const [pinned, setPinned] = useState(false);
  // Closed from inside with the keyboard sent back to the trigger: the
  // trigger's focus does not reopen it until the pointer or focus comes anew.
  const [dismissed, setDismissed] = useState(false);
  const anchor = useRef<HTMLSpanElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const agent = agentsByPane(useShellStore((state) => allAgents(state.agents, state.rest?.status?.remote))).get(paneId);
  const header = useShellStore((state) => state.rest?.terminal?.headers?.[paneId]);
  const open = !dismissed && (pinned || focused);
  const close = () => {
    setPinned(false);
    setFocused(false);
  };
  // The keyboard moves between the trigger and the card as one place, so the
  // card stays open while focus is in either; only a Tab moves it in, so the
  // pointer never takes the terminal's keyboard.
  const inside = (node: EventTarget | null) => node instanceof Node && (anchor.current?.contains(node) || content.current?.contains(node)) === true;
  const blur = (event: FocusEvent) => {
    if (inside(event.relatedTarget)) return;
    setFocused(false);
    setDismissed(false);
  };
  const focusable = () => [...(content.current?.querySelectorAll<HTMLElement>("button, a[href]") ?? [])];
  const toTrigger = () => anchor.current?.querySelector<HTMLElement>("button")?.focus();
  // Closing with the keyboard in the card hands it back to the trigger, which
  // keeps the card closed; any other close leaves the keyboard where it is.
  const dismiss = () => {
    const returns = content.current?.contains(document.activeElement) === true;
    close();
    if (!returns) return;
    setDismissed(true);
    toTrigger();
  };
  const state: TurnCardTrigger = {
    open,
    pinned,
    onFocus: () => setFocused(true),
    onBlur: blur,
    onKeyDown: (event) => {
      const first = focusable()[0];
      if (event.key !== "Tab" || event.shiftKey || !open || !first) return;
      event.preventDefault();
      first.focus();
    },
    onToggle: () => {
      setDismissed(false);
      if (pinned) close();
      else setPinned(true);
    },
  };
  const session = agent ? sessionOfAgent(agent) : null;
  return (
    <Popover open={open && agent !== undefined} onOpenChange={(next) => (next ? undefined : dismiss())} modal={false}>
      <PopoverAnchor asChild>
        <span ref={anchor} className="flex min-w-0 items-center">{trigger(state)}</span>
      </PopoverAnchor>
      {agent ? (
        <PopoverContent
          ref={content}
          side="bottom"
          align="start"
          collisionPadding={8}
          className="max-h-[min(var(--radix-popover-content-available-height),70dvh)] w-(--size-turn-card) overflow-y-auto p-0 text-caption"
          data-pane-turn-card={paneId}
          onOpenAutoFocus={(event) => event.preventDefault()}
          onCloseAutoFocus={(event) => event.preventDefault()}
          // A click on the card's text focuses the card itself, which is
          // not the keyboard moving in.
          onFocus={(event) => {
            if (event.target !== content.current) setFocused(true);
          }}
          onBlur={blur}
          onKeyDown={(event) => {
            // The card sits at the end of the page, so Shift+Tab before its
            // first control goes back to the trigger with the card open, and
            // Tab past its last closes it there, so the next Tab moves on.
            const items = focusable();
            const edge = event.shiftKey ? items[0] : items[items.length - 1];
            if (event.key !== "Tab" || document.activeElement !== edge) return;
            event.preventDefault();
            if (event.shiftKey) toTrigger();
            else dismiss();
          }}
          // The trigger is outside the card, so its own press would dismiss
          // the card before its click toggles the pin; the click alone decides.
          onInteractOutside={(event) => {
            if (event.target instanceof Node && anchor.current?.contains(event.target)) event.preventDefault();
          }}
          onEscapeKeyDown={(event) => {
            // Esc closes the card and stops there, so an agent CLI in the
            // focused terminal is not interrupted by it.
            event.stopPropagation();
            dismiss();
          }}
        >
          <TurnCard agent={agent} header={header} onReadWhole={(offset) => actions.readTurnAnswer(paneId, offset)} onRecord={session ? () => {
            close();
            actions.openSessionRecord(session);
          } : null} />
        </PopoverContent>
      ) : null}
    </Popover>
  );
}

/** A request's one line as the row and the card draw it: its words, else its image count. */
export function requestWords(t: TFunction<"translation">, request: PaneTurnRequest | { text: string | null; images: number }): string {
  const images = request.images > 0 ? t("requests.images", { count: request.images }) : null;
  return request.text ? (images ? `${request.text} · ${images}` : request.text) : images ?? "";
}

/** Who sent a request, in the row's words. */
export function senderName(t: TFunction<"translation">, sender: RequestSender): string {
  return sender.kind === "named" ? sender.name : t(sender.kind === "operator" ? "requests.sender.operator" : "requests.sender.agent");
}

/** The response half of a turn: the verb word, whether it waits on the operator, and what it says. */
export function turnResponse(t: TFunction<"translation">, header: PaneHeader | undefined, agent: AgentRow): { verb: string; asks: boolean; text: string | null } | null {
  const band = header?.band ?? null;
  if (band?.raised) return { verb: t(`agentSessions.verb.${band.raised.verb}`), asks: true, text: askWhat(t, band.raised.verb, band.raised.what, band.raised.subject) };
  if (band) {
    if (band.kind === "approval" || band.kind === "answer") return { verb: t(`agentSessions.verb.${band.kind}`), asks: true, text: askWhat(t, band.kind, band.reason, band.subject) };
    // Work left is not the operator's to answer, so it is not drawn as an ask.
    return band.kind === "blocked" ? { verb: t("agentSessions.tag.blocked"), asks: true, text: band.reason } : { verb: statusText(t, "unfinished"), asks: false, text: band.reason };
  }
  if (header?.finished) return { verb: t("paneTurn.finished"), asks: false, text: header.progress ?? null };
  if (header?.working) return { verb: statusText(t, "working"), asks: false, text: header.progress ?? null };
  const end = agent.request?.end;
  if (!end) return null;
  return { verb: t(`requests.end.${end}`), asks: end === "question" || end === "blocked", text: agent.request?.line ?? null };
}

/** A turn's place on the card's rail: a ring in its end's colour, filled for the shown turn's two halves; the answer filled in success, the letters a small ring. */
type RailDot = "past" | "past-asks" | "past-done" | "request" | "answer" | "letters" | Tone;

/** How the agent stands on the shown turn: waiting on the operator, working, finished or quiet. */
type Tone = "asks" | "working" | "done" | "quiet";

const DOT: Record<RailDot, string> = {
  past: "border-muted-foreground bg-popover",
  "past-asks": "border-warning bg-popover",
  "past-done": "border-success bg-popover",
  request: "border-foreground bg-foreground",
  answer: "border-success bg-success",
  letters: "border-muted-foreground bg-popover",
  asks: "border-warning bg-warning ring-3 ring-warning/25",
  working: "border-agent-working bg-agent-working ring-3 ring-agent-working/25",
  done: "border-success bg-success",
  quiet: "border-muted-foreground bg-muted-foreground",
};

/** The verb's pill in its tone's colour, so the state reads before its words. */
const TONE: Record<Tone, string> = {
  asks: "bg-warning/15 text-warning",
  working: "bg-agent-working/15 text-agent-working",
  done: "bg-success/15 text-success",
  quiet: "bg-secondary text-foreground",
};

/** One stop on the rail: its dot sits on the line at the left, its words to the right. */
function RailItem({ dot, className, children, ...rest }: { dot: RailDot; className?: string; children: ReactNode } & { [data: `data-${string}`]: string | number | undefined }) {
  return (
    <li className={cn("relative min-w-0", className)} {...rest}>
      <span aria-hidden="true" className={cn("absolute top-xs -left-lg size-(--size-rail-mark) rounded-full border-2", DOT[dot])} />
      {children}
    </li>
  );
}

function pastDot(end: LabelEnd | null): RailDot {
  if (end === "question" || end === "blocked") return "past-asks";
  return end === "done" ? "past-done" : "past";
}

/** A clock time on the card, in the operator's language. */
function Clock({ at }: { at: number }) {
  const { i18n } = useInterfaceTranslation();
  return (
    <time className="shrink-0 font-mono text-micro text-muted-foreground" dateTime={new Date(at).toISOString()}>
      {formatDateTime(requireInterfaceLanguage(i18n.language), at, { hour: "2-digit", minute: "2-digit", hour12: false })}
    </time>
  );
}

/** Which way a letter went, as a glyph in its direction's colour. */
const DIRECTION: Record<LetterLine["direction"], { glyph: string; tone: string }> = {
  sent: { glyph: "↗", tone: "text-agent-working" },
  received: { glyph: "↙", tone: "text-success" },
  started: { glyph: "+", tone: "text-subtle-foreground" },
};

/**
 * A letter's mark: a block and a report in their states' colours, the rest
 * quiet, since the warning colour is kept for what the operator answers.
 */
const MARK: Record<LetterMark, string> = {
  block: "bg-destructive/15 text-destructive",
  report: "bg-success/15 text-success",
  waiting_answer: "bg-secondary text-subtle-foreground",
  reply: "bg-secondary text-subtle-foreground",
  question: "bg-secondary text-subtle-foreground",
};

/** One line of the card's later half: glyph, who, mark, words and time, cut to the card's width. */
function CardRow({ glyph, who, mark, words, at, className, label, ...rest }: { glyph: ReactNode; who: string; mark?: ReactNode; words: ReactNode; at: number; className?: string; label: string } & { [data: `data-${string}`]: string | number | undefined }) {
  return (
    <li className={cn("flex min-w-0 items-baseline gap-xs", className)} aria-label={label} {...rest}>
      <span aria-hidden="true" className="w-(--size-icon-sm) shrink-0 text-center">{glyph}</span>
      <span aria-hidden="true" className="max-w-1/4 shrink-0 truncate text-muted-foreground">{who}</span>
      {mark}
      <span aria-hidden="true" className="min-w-0 flex-1 truncate">{words}</span>
      <Clock at={at} />
    </li>
  );
}

function LetterRow({ letter }: { letter: LetterLine }) {
  const { t } = useInterfaceTranslation();
  const who = senderName(t, letter.peer);
  const direction = DIRECTION[letter.direction];
  const words = letter.direction === "started" ? t("paneTurn.started") : letter.line ?? "";
  const markWord = letter.mark ? t(`paneTurn.mark.${letter.mark}`) : null;
  return (
    <CardRow
      glyph={<span className={direction.tone}>{direction.glyph}</span>}
      who={who}
      mark={letter.mark ? <span className={cn("shrink-0 rounded-xs px-xxs text-micro font-semibold", MARK[letter.mark])}>{markWord}</span> : null}
      words={<span className="text-foreground">{words}</span>}
      at={letter.at_unix_ms}
      label={[t(`paneTurn.direction.${letter.direction}`, { name: who }), markWord, words].filter(Boolean).join(" · ")}
      data-turn-card-letter={letter.direction}
    />
  );
}

function AskedRow({ asked }: { asked: AskedTurn }) {
  const { t } = useInterfaceTranslation();
  return (
    <CardRow
      glyph={<span className="font-semibold text-warning">!</span>}
      who={t("paneTurn.askedTo")}
      words={<span className="text-warning">{asked.line} · {t("paneTurn.unanswered")}</span>}
      at={asked.at_unix_ms}
      className="-mx-xs rounded-xs bg-warning/10 px-xs"
      label={`${t("paneTurn.askedTo")} · ${asked.line} · ${t("paneTurn.unanswered")}`}
      data-turn-card-asked="true"
    />
  );
}

/**
 * The agent's answer to the request: the label's line in bold and the first
 * lines of its own last message dimmed under it, or that message alone in
 * the foreground without a line. `전체 보기` opens the whole message in place,
 * read when opened (`turn_answer`) and drawn as Markdown, in a box of its own height so the letters
 * under it stay in view.
 */
function AnswerItem({ answer, pill, read, onReadWhole, opened = false }: { answer: TurnAnswer; pill: ReactNode; read: TurnAnswerRead | undefined; onReadWhole: (offset: number) => void; opened?: boolean }) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(opened);
  const mine = read?.offset === answer.offset;
  const phase = open && mine ? read?.phase : null;
  return (
    <RailItem dot={pill ? "done" : "answer"} data-turn-card-answer="true">
      <h3 className="sr-only">{t("paneTurn.answer")}</h3>
      <p className="flex items-center gap-xs text-micro text-muted-foreground">
        {pill ?? <span>{t("paneTurn.answer")}</span>}
        <Clock at={answer.at_unix_ms} />
      </p>
      <div className="mt-xxs flex min-w-0 flex-col gap-xxs rounded-md rounded-tl-xs bg-success/10 px-sm py-xs">
        {answer.line ? <p className="whitespace-normal font-semibold text-foreground [overflow-wrap:anywhere]" data-turn-card-answer-line="true">{answer.line}</p> : null}
        {answer.head && !(open && phase === "ready") ? (
          <p className={cn("line-clamp-2 whitespace-pre-line [overflow-wrap:anywhere]", answer.line ? "text-muted-foreground" : "text-foreground")} data-turn-card-answer-head="true">{answer.head}</p>
        ) : null}
        {open && phase === "ready" && read?.text ? (
          // The agent's words as Markdown, its tables and lists drawn as the
          // PR card draws a body, with no HTML of its own.
          <div className="flex max-h-(--size-turn-card-answer) flex-col gap-sm overflow-y-auto text-foreground" data-turn-card-answer-whole="true">
            {renderMarkdown(read.text)}
          </div>
        ) : null}
        {open && phase === "reading" ? <p className="text-muted-foreground">{t("paneTurn.wholeReading")}</p> : null}
        {open && phase === "failed" ? <p className="text-muted-foreground" data-turn-card-answer-failed="true">{t("paneTurn.wholeFailed")}</p> : null}
        {answer.whole ? (
          <Button
            variant="text"
            size="inline"
            className="self-start text-caption"
            aria-expanded={open}
            data-turn-card-whole={open ? "open" : "closed"}
            onClick={() => {
              if (!open) onReadWhole(answer.offset);
              setOpen(!open);
            }}
          >
            {open ? `${t("paneTurn.fold")} ▴` : `${t("paneTurn.whole")} ▾`}
          </Button>
        ) : null}
      </div>
    </RailItem>
  );
}

/**
 * The card's body: the session as one conversation read top to bottom on a
 * rail: the turns before (oldest first, dimmed, each ringed in how it ended),
 * the shown request in a bubble under who sent it and when (its summary and
 * its own first two lines, who asked after), the agent's answer to it once
 * that turn is over, the letters the agent sent and received and the agents
 * it started since, with the questions its later turns put to the operator in
 * their places, and the agent's half now: what it does as a coloured verb
 * with the label's words. A part with nothing to say is not drawn (B7).
 */
export function TurnCard({ agent, header, onRecord, onReadWhole, answerOpened }: { agent: AgentRow; header: PaneHeader | undefined; onRecord: (() => void) | null; onReadWhole: (offset: number) => void; /** The gallery's opened answer. */ answerOpened?: boolean }) {
  const { t } = useInterfaceTranslation();
  const read = useShellStore((state) => (state.rest?.turn_answer?.pane_id === agent.pane_id ? state.rest.turn_answer : undefined));
  const view = agent.request;
  const raw = view?.request ?? null;
  const summary = view?.request_summary ?? null;
  const response = turnResponse(t, header, agent);
  // The core sends the turns before newest first; the rail reads in time order.
  const earlier = [...(view?.recent_turns ?? [])].reverse();
  const original = raw?.text.trim() || null;
  const tone: Tone = response?.asks ? "asks" : header?.working ? "working" : header?.finished ? "done" : "quiet";
  const answer = view?.answer ?? null;
  // With no later turn and nothing asked, the answer is how the agent stands
  // now: it wears the verb, and no second line repeats it.
  const answerIsNow = answer !== null && !view?.later_by && response !== null && !response.asks && !header?.working;
  const lines = cardLines(view?.letters, view?.asked, response?.asks === true);
  const count = letterCount(view?.letters);
  const before = lettersBefore(view?.letters);
  const progress = view?.letters?.progress ?? 0;
  const later = lines.length > 0 || count > 0;
  const rail = raw !== null || response !== null || earlier.length > 0 || answer !== null || later;
  return (
    <div className="flex min-w-0 flex-col [word-break:keep-all]" data-turn-card="true">
      <div className="flex flex-col gap-sm px-sm pt-sm">
        {rail ? (
          <ol className="relative flex min-w-0 flex-col gap-sm pl-lg before:absolute before:top-sm before:bottom-sm before:left-xs before:w-px before:bg-muted-foreground/40">
            {earlier.length > 0 ? (
              <li className="min-w-0" data-turn-card-earlier={earlier.length}>
                <h3 className="sr-only">{t("paneTurn.earlier")}</h3>
                <ol className="flex flex-col gap-xs">
                  {earlier.map((turn, index) => (
                    <RailItem key={index} dot={pastDot(turn.end)} className="opacity-(--opacity-dimmed)" data-turn-card-earlier-item="true">
                      <p className="truncate text-foreground">{turn.request ?? t("paneTurn.unsummarized")}</p>
                      {turn.end || turn.line ? <p className="truncate text-muted-foreground">{[turn.end ? t(`requests.end.${turn.end}`) : null, turn.line].filter(Boolean).join(" · ")}</p> : null}
                    </RailItem>
                  ))}
                </ol>
              </li>
            ) : null}
            {raw ? (
              <RailItem dot="request" data-turn-card-request="true">
                <h3 className="sr-only">{t("paneTurn.request")}</h3>
                <p className="flex items-center gap-xs text-micro text-muted-foreground">
                  <span data-turn-card-sender="true">{senderName(t, raw.sender)}</span>
                  <Clock at={raw.at_unix_ms} />
                </p>
                <div className="mt-xxs flex min-w-0 flex-col gap-xxs rounded-md rounded-bl-xs bg-secondary px-sm py-xs">
                  <p className="whitespace-normal font-semibold text-foreground [overflow-wrap:anywhere]" data-turn-card-summary="true">{requestWords(t, { text: summary, images: raw.images })}</p>
                  {original && original !== summary ? <p className="line-clamp-2 whitespace-pre-line text-muted-foreground [overflow-wrap:anywhere]" data-turn-card-original="true">{original}</p> : null}
                </div>
                {view?.later_by ? <p className="mt-xxs text-micro text-muted-foreground" data-turn-card-later="true">{t("paneTurn.laterBy", { name: senderName(t, view.later_by) })}</p> : null}
              </RailItem>
            ) : null}
            {answer ? (
              <AnswerItem
                answer={answer}
                pill={answerIsNow && response ? <span className={cn("inline-flex items-center rounded-full px-xs font-semibold", TONE[tone])}>{response.verb}</span> : null}
                read={read}
                onReadWhole={onReadWhole}
                opened={answerOpened}
              />
            ) : null}
            {later ? (
              <RailItem dot="letters" data-turn-card-letters={count}>
                <h3 className="text-micro text-muted-foreground" data-turn-card-letters-head="true">{t("paneTurn.letters", { count })}</h3>
                <ol className="mt-xxs flex min-w-0 flex-col gap-xxs">
                  {before > 0 ? <li className="text-muted-foreground" data-turn-card-letters-before={before}>{t("paneTurn.lettersEarlier", { count: before })}</li> : null}
                  {lines.map((line, index) => (line.kind === "letter" ? <LetterRow key={index} letter={line.letter} /> : <AskedRow key={index} asked={line.asked} />))}
                  {progress > 0 ? <li className="text-muted-foreground" data-turn-card-progress={progress}>{t("paneTurn.progressFolded", { count: progress })}</li> : null}
                </ol>
              </RailItem>
            ) : null}
            {response && !answerIsNow ? (
              <RailItem dot={tone}>
                <p className="min-w-0 whitespace-normal [overflow-wrap:anywhere]" data-turn-card-response="true">
                  <span className={cn("mr-xs inline-flex items-center rounded-full px-xs font-semibold", TONE[tone])}>{response.verb}</span>
                  {response.text ? <span className={response.asks ? "text-warning" : "text-foreground"}>{response.text}</span> : null}
                </p>
              </RailItem>
            ) : null}
          </ol>
        ) : null}
      </div>
      {onRecord ? (
        <div className="mt-sm border-t border-border p-xxs">
          <button type="button" className="flex w-full items-center rounded-xs px-xs py-xs text-left outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring" onClick={onRecord} data-turn-card-record="true">
            {t("sessionWork.openRecord")}
          </button>
        </div>
      ) : <div className="pb-sm" />}
    </div>
  );
}
