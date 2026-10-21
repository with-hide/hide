import { useEffect, useRef, useState, type ReactNode } from "react";
import type { TFunction } from "i18next";
import type { Actions } from "./actions";
import { askWhat } from "./components/agent-tree";
import { Popover, PopoverAnchor, PopoverContent } from "./components/ui/popover";
import { statusText } from "./agentStatus";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import { agentsByPane, allAgents } from "./paneAgents";
import { sessionOfAgentOn } from "./sessionWork";
import { localDeviceId, type AgentRow, type PaneHeader, type PaneTurnRequest, type RequestSender } from "./snapshot";
import { useShellStore } from "./store";

/** How long the pointer may cross the gap between a trigger and its card before the card closes. */
const LEAVE_MS = 150;

/** The handlers a turn card's trigger wears; `pinned` is set only on a trigger that pins. */
export type TurnCardTrigger = {
  open: boolean;
  pinned: boolean;
  onPointerEnter: () => void;
  onPointerLeave: () => void;
  onFocus: () => void;
  onBlur: () => void;
  /** Pins or unpins the card; absent on a trigger whose press does something else. */
  onToggle?: () => void;
  /** Closes the card, for a trigger whose press opens something in its place. */
  close: () => void;
};

/**
 * The pane's turn card (PRD pane-last-request D-05, D-12, B6-B9): what the
 * operator asked, what the agent is doing about it, its last words and the
 * turns before, under the pane's head. The pointer or the keyboard's focus
 * on its trigger opens it; a trigger that pins takes a press to pin it and a
 * second press or Esc to close it. Opening never takes the keyboard from the
 * terminal, and Esc on a card neither pinned nor focused stays the terminal's.
 */
export function PaneTurnCard({ paneId, actions, pinnable, head, trigger }: { paneId: string; actions: Actions; pinnable: boolean; head?: { title: string; product: string; words: string | null }; trigger: (state: TurnCardTrigger) => ReactNode }) {
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const [pinned, setPinned] = useState(false);
  const leave = useRef<number | null>(null);
  const anchor = useRef<HTMLSpanElement>(null);
  useEffect(() => () => {
    if (leave.current !== null) window.clearTimeout(leave.current);
  }, []);
  const agent = agentsByPane(useShellStore((state) => allAgents(state.agents, state.rest?.status?.remote))).get(paneId);
  const header = useShellStore((state) => state.rest?.terminal?.headers?.[paneId]);
  const localId = useShellStore((state) => localDeviceId(state.rest));
  const open = pinned || hovered || focused;
  const enter = () => {
    if (leave.current !== null) window.clearTimeout(leave.current);
    leave.current = null;
    setHovered(true);
  };
  const exit = () => {
    if (leave.current !== null) window.clearTimeout(leave.current);
    leave.current = window.setTimeout(() => setHovered(false), LEAVE_MS);
  };
  const close = () => {
    setPinned(false);
    setHovered(false);
    setFocused(false);
  };
  const state: TurnCardTrigger = {
    open,
    pinned,
    onPointerEnter: enter,
    onPointerLeave: exit,
    onFocus: () => setFocused(true),
    onBlur: () => setFocused(false),
    onToggle: pinnable ? () => (pinned ? close() : setPinned(true)) : undefined,
    close,
  };
  const session = agent ? sessionOfAgentOn(agent, localId) : null;
  return (
    <Popover open={open && agent !== undefined} onOpenChange={(next) => (next ? undefined : close())} modal={false}>
      <PopoverAnchor asChild>
        <span ref={anchor} className="flex min-w-0 items-center">{trigger(state)}</span>
      </PopoverAnchor>
      {agent ? (
        <PopoverContent
          side="bottom"
          align="start"
          collisionPadding={8}
          className="max-h-(--radix-popover-content-available-height) overflow-y-auto p-0 text-caption"
          data-pane-turn-card={paneId}
          onOpenAutoFocus={(event) => event.preventDefault()}
          onCloseAutoFocus={(event) => event.preventDefault()}
          onPointerEnter={enter}
          onPointerLeave={exit}
          // The trigger is outside the card, so its own press would dismiss
          // the card before its click toggles the pin; the click alone decides.
          onInteractOutside={(event) => {
            if (event.target instanceof Node && anchor.current?.contains(event.target)) event.preventDefault();
          }}
          onEscapeKeyDown={(event) => {
            // Esc closes a pinned or focused card and stops there, so an
            // agent CLI in the focused terminal is not interrupted by it; a
            // card the pointer only shows leaves Esc to the terminal.
            if (!pinned && !focused) return event.preventDefault();
            event.stopPropagation();
            close();
          }}
        >
          <TurnCard agent={agent} header={header} head={head} onRecord={session ? () => {
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
function senderName(t: TFunction<"translation">, sender: RequestSender): string {
  return sender.kind === "named" ? sender.name : t(sender.kind === "operator" ? "requests.sender.operator" : "requests.sender.agent");
}

/** The response half of a turn: the verb word, whether it waits on the operator, and what it says. */
export function turnResponse(t: TFunction<"translation">, header: PaneHeader | undefined, agent: AgentRow): { verb: string; asks: boolean; text: string | null } | null {
  const band = header?.band ?? null;
  if (band?.raised) return { verb: t(`agentSessions.verb.${band.raised.verb}`), asks: true, text: askWhat(t, band.raised.verb, band.raised.what) };
  if (band) {
    if (band.kind === "approval" || band.kind === "answer") return { verb: t(`agentSessions.verb.${band.kind}`), asks: true, text: askWhat(t, band.kind, band.reason) };
    return { verb: t(band.kind === "blocked" ? "agentSessions.tag.blocked" : "agentSessions.tag.stopped"), asks: true, text: band.reason };
  }
  if (header?.finished) return { verb: t("paneTurn.finished"), asks: false, text: header.progress ?? null };
  if (header?.working) return { verb: statusText(t, "working"), asks: false, text: header.progress ?? null };
  const end = agent.request?.end;
  if (!end) return null;
  return { verb: t(`requests.end.${end}`), asks: end === "question" || end === "blocked", text: agent.request?.line ?? null };
}

/**
 * The card's body: the head the title's hint carried, the request (its
 * summary and its own first two lines), what the agent does about it, its
 * last words, who asked after, the turns before and `기록 열기`. A part with
 * nothing to say is not drawn (B7).
 */
function TurnCard({ agent, header, head, onRecord }: { agent: AgentRow; header: PaneHeader | undefined; head?: { title: string; product: string; words: string | null }; onRecord: (() => void) | null }) {
  const { t } = useInterfaceTranslation();
  const view = agent.request;
  const raw = view?.request ?? null;
  const summary = view?.request_summary ?? null;
  const response = turnResponse(t, header, agent);
  const reply = view?.reply?.text.trim() || null;
  const earlier = view?.recent_turns ?? [];
  const original = raw?.text.trim() || null;
  return (
    <div className="flex min-w-0 flex-col [word-break:keep-all]" data-turn-card="true">
      <div className="flex flex-col gap-sm px-sm pt-sm">
        {head ? (
          <div className="flex min-w-0 flex-col" data-turn-card-head="true">
            <span className="truncate font-semibold text-foreground">{head.title}</span>
            <span className="truncate text-micro text-muted-foreground">{[head.product, head.words].filter(Boolean).join(" · ")}</span>
          </div>
        ) : null}
        {raw ? (
          <section className="flex min-w-0 flex-col gap-xxs" data-turn-card-request="true">
            <h3 className="text-micro font-medium text-muted-foreground">{t("paneTurn.request")}</h3>
            <p className="whitespace-normal text-foreground [overflow-wrap:anywhere]">{requestWords(t, { text: summary, images: raw.images })}</p>
            {original && original !== summary ? <p className="line-clamp-2 whitespace-pre-line text-muted-foreground [overflow-wrap:anywhere]" data-turn-card-original="true">{original}</p> : null}
            {view?.later_by ? <p className="text-micro text-muted-foreground" data-turn-card-later="true">{t("paneTurn.laterBy", { name: senderName(t, view.later_by) })}</p> : null}
          </section>
        ) : null}
        {response ? (
          <p className="min-w-0 whitespace-normal [overflow-wrap:anywhere]" data-turn-card-response="true">
            <span className={cn("mr-xs font-semibold", response.asks ? "text-warning" : "text-foreground")}>{response.verb}</span>
            {response.text ? <span className={response.asks ? "text-warning" : "text-muted-foreground"}>{response.text}</span> : null}
          </p>
        ) : null}
        {reply ? (
          <section className="flex min-w-0 flex-col gap-xxs" data-turn-card-reply="true">
            <h3 className="text-micro font-medium text-muted-foreground">{t("paneTurn.reply")}</h3>
            <p className="line-clamp-3 whitespace-pre-line text-muted-foreground [overflow-wrap:anywhere]">{reply}</p>
          </section>
        ) : null}
        {earlier.length > 0 ? (
          <section className="flex min-w-0 flex-col gap-xxs" data-turn-card-earlier={earlier.length}>
            <h3 className="text-micro font-medium text-muted-foreground">{t("paneTurn.earlier")}</h3>
            <ul className="flex flex-col gap-xxs">
              {earlier.map((turn, index) => (
                <li key={index} className="min-w-0 truncate text-muted-foreground" data-turn-card-earlier-item="true">
                  <span className="text-foreground">{turn.request ?? t("paneTurn.unsummarized")}</span>
                  {turn.end ? <span> · {t(`requests.end.${turn.end}`)}</span> : null}
                  {turn.line ? <span> · {turn.line}</span> : null}
                </li>
              ))}
            </ul>
          </section>
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
