import { CheckIcon, CircleAlertIcon } from "lucide-react";
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import type { Actions } from "./actions";
import { useElapsed } from "./components/elapsed";
import { StatusMark } from "./components/status-mark";
import { Button } from "./components/ui/button";
import { Input } from "./components/ui/input";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import type { Mission } from "./snapshot";
import { composingKey } from "./keys";
import { ANSWER_LIMIT, answerView, reasonKey, usePressedAnswer, writtenAnswer, type PressedAnswer } from "./missionAnswer";

// The decision a mission waits on, answered where it is asked, first in the
// page's 내 차례 (PRD mission-plan-visibility D-13, D-20, D-30, B25-B30, B39;
// PRD mission-map-git-criteria B25): the question, why
// it matters, the node it stopped and how long it waited, the Conductor's
// choices with its recommendation, a short written answer, and the way to
// the conversation. The core keeps the answer and types it into the
// Conductor's conversation as the operator's own input; this draws where
// that answer is. One typed without Herdr confirming it is typed again only
// when the operator sends it again, so a question is answered once (B26).

export function DecisionBanner({ mission, device, actions, onNode }: { mission: Mission; device: string | null; actions: Actions; onNode: (key: string) => void }) {
  const { t } = useInterfaceTranslation();
  const question = mission.question;
  const [pressed, setPressed] = useState<PressedAnswer | null>(null);
  const [writing, setWriting] = useState(false);
  const [draft, setDraft] = useState("");
  const writeButton = useRef<HTMLButtonElement>(null);
  const field = useRef<HTMLInputElement>(null);
  const returnFocus = useRef(false);
  // While the mission's device does not answer, the wait is not counted on (B35).
  const waited = useElapsed(mission.stale ? null : (question?.asked_at_unix_ms ?? null));
  const stopped = question?.node ? (mission.nodes.find((node) => node.key === question.node) ?? null) : null;
  const view = answerView(mission, device, pressed);
  const { key, options, sent, unreachable, closed } = view;

  // What was pressed shows until the core answers for it, refuses it, or asks something else (B26).
  usePressedAnswer(mission, pressed, view, () => setPressed(null));
  useEffect(() => {
    if (writing) field.current?.focus();
    else if (returnFocus.current) {
      returnFocus.current = false;
      writeButton.current?.focus();
    }
  }, [writing]);

  const choose = (index: number) => {
    actions.answerMission(mission.id, key, { option: index + 1 });
    setPressed({ key, text: options[index]!, failed: mission.answer_failed, resend: false });
  };
  const resend = () => {
    if (!sent) return;
    actions.answerMission(mission.id, key, { resend: true });
    setPressed({ key, text: sent.text, failed: mission.answer_failed, resend: true });
  };
  const onKey = (event: KeyboardEvent<HTMLInputElement>) => {
    if (composingKey(event)) return;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      returnFocus.current = true;
      setWriting(false);
    } else if (event.key === "Enter") {
      event.preventDefault();
      const text = writtenAnswer(draft);
      if (!text) return;
      actions.answerMission(mission.id, key, { text });
      setPressed({ key, text, failed: mission.answer_failed, resend: false });
      setWriting(false);
      setDraft("");
    }
  };

  // Why the operator cannot answer now, or why the last answer did not arrive (B28, B39).
  const reason = view.reason ? t(reasonKey(view.reason), { device }) : null;

  return (
    <div
      role="group"
      aria-label={mission.decision ?? undefined}
      className="mx-md mb-xs flex min-w-0 flex-col gap-xs rounded-md border border-mission-hold bg-card px-sm py-xs"
      data-mission-banner="decision"
      data-mission-answer={sent?.state ?? (closed ? "closed" : "open")}
    >
      <div className="flex min-w-0 items-center gap-sm">
        <StatusMark mark="turn" />
        <span className="min-w-0 flex-1 truncate text-body font-semibold" title={mission.decision ?? undefined} data-mission-question="true">{mission.decision}</span>
        {stopped ? (
          // The node chip picks its node on the map below (PRD mission-map-git-criteria B25).
          <button type="button" onClick={() => onNode(stopped.key)} className="max-w-(--size-flow-title) shrink-0 truncate rounded-xs bg-muted px-xs text-caption text-mission-hold outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring" title={t("mission.banner.stopped", { node: stopped.title })} data-mission-banner-node={stopped.key}>
            {t("mission.banner.stopped", { node: stopped.title })}
          </button>
        ) : null}
        {waited ? <span className="shrink-0 text-caption text-muted-foreground tabular-nums" data-mission-banner-waited="true">{t("mission.banner.waited", { elapsed: waited })}</span> : null}
        {/* The Conductor row just above carries 터미널로 ›, or 이어받기 with no Conductor (PRD mission-tool B12, B13). */}
      </div>
      {question?.why ? <p className="min-w-0 break-words text-caption text-muted-foreground" data-mission-banner-why="true">{question.why}</p> : null}
      {sent?.line === "unconfirmed" ? (
        <p className="flex min-w-0 items-center gap-xs text-body" data-mission-banner-sent="unconfirmed">
          <CircleAlertIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-mission-hold" />
          <span className="min-w-0 truncate font-semibold" title={sent.text}>{sent.text}</span>
          <span className="shrink-0 text-caption text-mission-hold">· {t(sent.word)}</span>
          <Button size="sm" variant="ghost" disabled={unreachable} onClick={resend} data-mission-resend="true">{t("mission.banner.resend")}</Button>
        </p>
      ) : sent?.line === "unreadable" ? (
        // The answer waits on a Conductor session Hide cannot read: it is not delivered until a read lands.
        <p className="flex min-w-0 items-center gap-xs text-body" data-mission-banner-sent={sent.state}>
          <CircleAlertIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-mission-failed" />
          <span className="min-w-0 truncate font-semibold" title={sent.text}>{sent.text}</span>
          <span className="min-w-0 truncate text-caption text-mission-failed" data-mission-banner-reason="session_unreadable">· {t(sent.word)}</span>
        </p>
      ) : sent ? (
        <p className="flex min-w-0 items-center gap-xs text-body" data-mission-banner-sent={sent.state}>
          <CheckIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-mission-done" />
          <span className="min-w-0 truncate font-semibold text-mission-done" title={sent.text}>{sent.text}</span>
          <span className="shrink-0 text-caption text-muted-foreground">· {t(sent.word)}</span>
        </p>
      ) : (
        <div className="flex min-w-0 flex-wrap items-center gap-xs">
          {options.map((option, index) => {
            const recommended = question?.recommend === index + 1;
            return (
              <Button
                key={index}
                size="sm"
                variant="secondary"
                disabled={closed}
                onClick={() => choose(index)}
                className={recommended ? "max-w-full font-semibold" : "max-w-full"}
                title={option}
                data-mission-option={index + 1}
                data-mission-recommended={recommended || undefined}
              >
                <span className="min-w-0 truncate">{option}</span>
                {recommended ? <span className="shrink-0 rounded-xs bg-background px-xxs text-micro text-foreground">{t("mission.banner.recommended")}</span> : null}
              </Button>
            );
          })}
          {writing ? (
            <span className="flex min-w-0 flex-1 basis-(--size-flow-title) items-center gap-xs">
              <Input
                ref={field}
                aria-label={t("mission.banner.input")}
                maxLength={ANSWER_LIMIT}
                value={draft}
                onChange={(event) => setDraft(event.target.value)}
                onKeyDown={onKey}
                className="h-(--size-control-sm)"
                data-mission-answer-input="true"
              />
              <span className="shrink-0 text-caption text-muted-foreground tabular-nums" data-mission-answer-left={ANSWER_LIMIT - draft.length}>
                {t("mission.banner.left", { count: ANSWER_LIMIT - draft.length })}
              </span>
            </span>
          ) : (
            <Button ref={writeButton} size="sm" variant="ghost" disabled={closed} onClick={() => setWriting(true)} data-mission-write="true">{t("mission.banner.write")}</Button>
          )}
        </div>
      )}
      {view.reason ? <p className={cn("min-w-0 text-caption", view.refused ? "text-mission-failed" : "text-muted-foreground")} data-mission-banner-reason={view.reason.kind === "failed" ? view.reason.failure : view.reason.kind}>{reason}</p> : null}
    </div>
  );
}
