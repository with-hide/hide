import { CheckIcon, ChevronRightIcon, CircleAlertIcon } from "lucide-react";
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import type { Actions } from "./actions";
import { useElapsed } from "./components/elapsed";
import { StatusMark } from "./components/status-mark";
import { Button } from "./components/ui/button";
import { Input } from "./components/ui/input";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import type { Mission, MissionAnswerState } from "./snapshot";

// The decision a mission waits on, answered where it is asked (PRD
// mission-plan-visibility D-13, D-20, D-30, B25-B30, B39): the question, why
// it matters, the node it stopped and how long it waited, the Conductor's
// choices with its recommendation, a short written answer, and the way to
// the conversation. The core keeps the answer and types it into the
// Conductor's conversation as the operator's own input; this draws where
// that answer is. One typed without Herdr confirming it is typed again only
// when the operator sends it again, so a question is answered once (B26).

/** The longest written answer the core takes (`missions::ANSWER_LIMIT`), counted in UTF-16 units so the field's own cap never lets past what the core refuses. */
export const ANSWER_LIMIT = 500;

/** How long a pressed answer reads as sent before the core's own answer arrives; past it the banner draws the core's state again. */
const PRESSED_MS = 5_000;

export function DecisionBanner({ mission, device, actions }: { mission: Mission; device: string | null; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const question = mission.question;
  // A decision from before questions is answered under key 0, as the core keeps it.
  const key = question?.key ?? 0;
  // `failed` is the failure shown when it was pressed, so only a new one ends it.
  const [pressed, setPressed] = useState<{ key: number; text: string; failed: Mission["answer_failed"]; resend: boolean } | null>(null);
  const [writing, setWriting] = useState(false);
  const [draft, setDraft] = useState("");
  const writeButton = useRef<HTMLButtonElement>(null);
  const field = useRef<HTMLInputElement>(null);
  const returnFocus = useRef(false);
  const waited = useElapsed(question?.asked_at_unix_ms ?? null);
  const stopped = question?.node ? (mission.nodes.find((node) => node.key === question.node) ?? null) : null;
  const answer = mission.answer?.key === key ? mission.answer : null;
  // Sending an unconfirmed answer again shows it sending until the core moves it on.
  const resending = pressed?.resend === true && answer?.state === "unconfirmed";

  // What was pressed shows until the core answers for it, refuses it, or asks something else (B26).
  useEffect(() => {
    if (!pressed) return;
    if ((answer && !resending) || pressed.key !== key || pressed.failed !== mission.answer_failed) return setPressed(null);
    const timer = window.setTimeout(() => setPressed(null), PRESSED_MS);
    return () => window.clearTimeout(timer);
  }, [pressed, answer, resending, key, mission.answer_failed]);
  useEffect(() => {
    if (writing) field.current?.focus();
    else if (returnFocus.current) {
      returnFocus.current = false;
      writeButton.current?.focus();
    }
  }, [writing]);

  const sent: { text: string; state: MissionAnswerState } | null = answer && !resending ? { text: answer.text, state: answer.state } : pressed ? { text: pressed.text, state: "sending" } : null;
  const gone = !mission.conductor.present;
  const offline = mission.stale;
  const unsupported = !mission.conductor.takes_answers;
  const unreachable = gone || offline || unsupported;
  const closed = unreachable || sent !== null;
  const options = question?.options ?? [];

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
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      returnFocus.current = true;
      setWriting(false);
    } else if (event.key === "Enter") {
      event.preventDefault();
      // The core takes one line: a tab or another control character becomes a space.
      const text = draft.replace(/\p{Cc}/gu, " ").trim();
      if (!text) return;
      actions.answerMission(mission.id, key, { text });
      setPressed({ key, text, failed: mission.answer_failed, resend: false });
      setWriting(false);
      setDraft("");
    }
  };

  // Why the operator cannot answer now, or why the last answer did not arrive (B28, B39).
  const reason = offline && device
    ? t("mission.banner.offline", { device })
    : !sent && mission.answer_failed
      ? t(`mission.banner.failed.${mission.answer_failed}`)
      : gone
        ? t("mission.noConductor")
        : unsupported
          ? t("mission.banner.unsupported")
          : null;

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
          <span className="max-w-(--size-flow-title) shrink-0 truncate rounded-xs bg-muted px-xs text-caption text-mission-hold" title={t("mission.banner.stopped", { node: stopped.title })} data-mission-banner-node={stopped.key}>
            {t("mission.banner.stopped", { node: stopped.title })}
          </span>
        ) : null}
        {waited ? <span className="shrink-0 text-caption text-muted-foreground tabular-nums" data-mission-banner-waited="true">{t("mission.banner.waited", { elapsed: waited })}</span> : null}
        {gone ? (
          <Button size="sm" onClick={() => actions.takeOverMission(mission.id)} data-mission-takeover="true">{t("mission.takeOver")}</Button>
        ) : (
          <Button size="sm" variant="ghost" onClick={() => actions.openAgent(mission.conductor.pane_id)} data-mission-conversation="true">
            {t("mission.banner.conversation")}
            <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" />
          </Button>
        )}
      </div>
      {question?.why ? <p className="min-w-0 break-words text-caption text-muted-foreground" data-mission-banner-why="true">{question.why}</p> : null}
      {sent?.state === "unconfirmed" ? (
        <p className="flex min-w-0 items-center gap-xs text-body" data-mission-banner-sent="unconfirmed">
          <CircleAlertIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-mission-hold" />
          <span className="min-w-0 truncate font-semibold" title={sent.text}>{sent.text}</span>
          <span className="shrink-0 text-caption text-mission-hold">· {t("mission.banner.unconfirmed")}</span>
          <Button size="sm" variant="ghost" disabled={unreachable} onClick={resend} data-mission-resend="true">{t("mission.banner.resend")}</Button>
        </p>
      ) : sent ? (
        <p className="flex min-w-0 items-center gap-xs text-body" data-mission-banner-sent={sent.state}>
          <CheckIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-mission-done" />
          <span className="min-w-0 truncate font-semibold text-mission-done" title={sent.text}>{sent.text}</span>
          <span className="shrink-0 text-caption text-muted-foreground">· {t(sent.state === "waiting" ? "mission.banner.waitingTurn" : "mission.banner.reading")}</span>
        </p>
      ) : (
        <div className="flex min-w-0 flex-wrap items-center gap-xs">
          {options.map((option, index) => {
            const recommended = question?.recommend === index + 1;
            return (
              <Button
                key={index}
                size="sm"
                variant={recommended ? "default" : "secondary"}
                disabled={closed}
                onClick={() => choose(index)}
                className="max-w-full"
                title={option}
                data-mission-option={index + 1}
                data-mission-recommended={recommended || undefined}
              >
                <span className="min-w-0 truncate">{option}</span>
                {recommended ? <span className="shrink-0 rounded-xs bg-background/20 px-xxs text-micro">{t("mission.banner.recommended")}</span> : null}
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
      {reason ? <p className={cn("min-w-0 text-caption", !sent && mission.answer_failed && !offline ? "text-mission-failed" : "text-muted-foreground")} data-mission-banner-reason={offline ? "offline" : (mission.answer_failed ?? (gone ? "no_conductor" : unsupported ? "unsupported" : undefined))}>{reason}</p> : null}
    </div>
  );
}
