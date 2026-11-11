import { CircleHelpIcon } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { Button } from "./ui/button";

/** A question the agent put to the operator that still waits: the record's id, its words and when it was asked. */
export type AskLine = { id: string; text: string; at_unix_ms: number };

/**
 * 나를 기다리는 것 (session-record-goal-flow D-12; status-model.md, A question
 * still waiting): the agent's open questions, oldest first, each with 무시하기.
 * A line put away keeps its place with 되돌리기 while the list stays drawn, and
 * leaves that state when the core lists it as open again after it was gone.
 * The session record and the turn card draw this one list, each in its own
 * frame, which is drawn while a line is, an open one or one put away; the
 * record marks an open line in the warning colour, and the turn card, which
 * keeps the list quiet beside the pane, in its own (`mark`).
 */
export function OpenAskList({ asks, onSet, time, frame, mark = "text-warning" }: {
  asks: readonly AskLine[];
  onSet: (id: string, status: "open" | "dismissed") => void;
  /** Draws when each was asked at the line's end (the turn card). */
  time?: (at: number) => ReactNode;
  /** The list in its place, with how many still wait. */
  frame: (list: ReactNode, open: number) => ReactNode;
  /** The colour of an open line's question mark. */
  mark?: string;
}) {
  const { t } = useInterfaceTranslation();
  // `gone` is set once the core no longer lists the line; listed again after that, it is open again.
  const [away, setAway] = useState<readonly { ask: AskLine; gone: boolean }[]>([]);
  useEffect(() => {
    const open = new Set(asks.map((ask) => ask.id));
    setAway((held) => {
      const next = held.filter((one) => !(one.gone && open.has(one.ask.id))).map((one) => (one.gone || open.has(one.ask.id) ? one : { ...one, gone: true }));
      return next.length === held.length && next.every((one, index) => one === held[index]) ? held : next;
    });
  }, [asks]);
  const lines = [...asks, ...away.map((one) => one.ask).filter((ask) => !asks.some((open) => open.id === ask.id))].sort((a, b) => a.at_unix_ms - b.at_unix_ms);
  if (!lines.length) return null;
  const small = "h-auto shrink-0 px-xs py-0 text-micro";
  return frame(
    <ul className="min-w-0">
      {lines.map((ask) => {
        const dismissed = away.some((one) => one.ask.id === ask.id);
        return (
          <li key={ask.id} className="flex min-w-0 items-start gap-xs border-b border-border py-xs last:border-b-0" data-record-ask={ask.id} data-dismissed={dismissed ? "true" : undefined}>
            <CircleHelpIcon aria-hidden="true" className={cn("mt-xxs size-(--size-icon-sm) shrink-0", dismissed ? "text-muted-foreground" : mark)} />
            <span className={cn("min-w-0 flex-1 whitespace-normal [overflow-wrap:anywhere]", dismissed ? "text-muted-foreground line-through" : "text-foreground")} title={ask.text}>{ask.text}</span>
            {time ? time(ask.at_unix_ms) : null}
            {dismissed ? (
              <Button size="sm" variant="ghost" className={small} onClick={() => { onSet(ask.id, "open"); setAway((held) => held.filter((one) => one.ask.id !== ask.id)); }} data-record-ask-undo={ask.id}>{t("backlog.panel.undo")}</Button>
            ) : (
              <Button size="sm" variant="ghost" className={small} onClick={() => { onSet(ask.id, "dismissed"); setAway((held) => [...held, { ask, gone: false }]); }} data-record-ask-dismiss={ask.id}>{t("sessionWork.dismiss")}</Button>
            )}
          </li>
        );
      })}
    </ul>,
    asks.length,
  );
}
