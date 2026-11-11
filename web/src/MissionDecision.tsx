import { CheckIcon, CircleAlertIcon, FileDiffIcon, FilePlusIcon, GlobeIcon, PencilIcon, ThumbsUpIcon } from "lucide-react";
import { Fragment, useEffect, useRef, useState, type KeyboardEvent } from "react";
import type { Actions } from "./actions";
import { useElapsed, useMinuteClock } from "./components/elapsed";
import { FileMark } from "./components/file-mark";
import { StatusMark } from "./components/status-mark";
import { Button } from "./components/ui/button";
import { Input } from "./components/ui/input";
import { RadioGroup, RadioGroupItem } from "./components/ui/radio-group";
import { RowButton } from "./components/ui/row-button";
import { fileIcon } from "./fileIcons";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import type { DecisionView, Mission } from "./snapshot";
import { composingKey } from "./keys";
import { ANSWER_LIMIT, answerView, reasonKey, usePressedAnswer, writtenAnswer, type PressedAnswer } from "./missionAnswer";
import { DECISION_VIEWS_SHOWN, gateAsked, gateWaitedLong } from "./missions";

// The decision a mission waits on, answered where it is asked, first in the
// page's 내 차례 (PRD mission-plan-visibility D-13, D-20, D-30, B25-B30, B39;
// PRD mission-map-git-criteria B25): the question, why
// it matters, the node it stopped and how long it waited, the Conductor's
// choices with its recommendation, a short written answer, and the way to
// the conversation. A choice is a radio row that a press only marks, and
// `답하기` sends it (PRD mission-human-gate D-19, B19). The core keeps the answer and types it into the
// Conductor's conversation as the operator's own input; this draws where
// that answer is. One typed without the engine confirming it is typed again only
// when the operator sends it again, so a question is answered once (B26).

export function DecisionBanner({ mission, device, actions, onNode }: { mission: Mission; device: string | null; actions: Actions; onNode: (key: string) => void }) {
  const { t } = useInterfaceTranslation();
  const question = mission.question;
  const [pressed, setPressed] = useState<PressedAnswer | null>(null);
  const [writing, setWriting] = useState(false);
  const [picked, setPicked] = useState<number | null>(null);
  const [draft, setDraft] = useState("");
  const writeButton = useRef<HTMLButtonElement>(null);
  const field = useRef<HTMLInputElement>(null);
  const returnFocus = useRef(false);
  // While the mission's device does not answer, the wait is not counted on (B35).
  const waited = useElapsed(mission.stale ? null : (question?.asked_at_unix_ms ?? null));
  // A gate asked for more than 24 hours says so beside its wait, in grey; nothing passes on its own (PRD mission-human-gate D-22, B22).
  const minute = useMinuteClock();
  const long = gateWaitedLong(gateAsked(mission)?.gate, minute);
  const stopped = question?.node ? (mission.nodes.find((node) => node.key === question.node) ?? null) : null;
  // The other nodes it is about, in the order the Conductor named them (mission-49: one question for seven merged nodes).
  const others = (question?.nodes ?? []).flatMap((key) => mission.nodes.filter((node) => node.key === key));
  const view = answerView(mission, device, pressed);
  const views = mission.decision_views ?? [];
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
      className="mx-md mb-xs flex min-w-0 flex-col gap-sm rounded-md border border-mission-hold bg-card px-md py-sm"
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
        {long ? <span className="shrink-0 text-caption text-muted-foreground" data-mission-banner-long="true">· {t("mission.gate.long")}</span> : null}
        {/* The Conductor row just above carries 터미널로 ›, or 이어받기 with no Conductor (PRD mission-tool B12, B13). */}
      </div>
      {question?.why ? <p className="min-w-0 break-words text-caption text-muted-foreground" data-mission-banner-why="true">{question.why}</p> : null}
      {others.length > 0 ? (
        <div className="flex min-w-0 flex-wrap items-center gap-xs text-caption text-muted-foreground" data-mission-banner-also={others.length}>
          <span className="shrink-0">{t("mission.banner.also")}</span>
          {others.map((node, index) => (
            <Fragment key={node.key}>
            {index > 0 ? <span aria-hidden="true">·</span> : null}
            <Button variant="text" size="inline" onClick={() => onNode(node.key)} className="max-w-(--size-flow-title) truncate text-caption text-foreground underline-offset-2 hover:underline" title={node.title} data-mission-banner-also-node={node.key}>
              {node.title}
            </Button>
            </Fragment>
          ))}
        </div>
      ) : null}
      {views.length > 0 ? <DecisionViews views={views} onOpen={(linked) => actions.openDecisionView(mission.id, key, linked)} /> : null}
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
          <span className="min-w-0 truncate text-caption text-muted-foreground" data-mission-banner-hold={mission.answer?.hold ?? "other"}>· {t(sent.word)}</span>
          {sent.canNow ? <Button size="sm" variant="ghost" onClick={() => actions.answerMission(mission.id, key, { now: true })} data-mission-now="true">{t("mission.banner.now")}</Button> : null}
        </p>
      ) : (
        <div className="flex min-w-0 flex-col gap-sm">
          {/* With nothing linked to look at, asking for material is a row of its own above the choices; it never holds the answer back. */}
          {views.length === 0 && question && !writing ? (
            question.material_asked ? (
              <span className="flex min-w-0 items-center gap-xs self-start text-caption text-muted-foreground" data-mission-material="asked">
                <CheckIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-mission-done" />
                {t("mission.banner.asked")}
              </span>
            ) : (
              <Button variant="outline" size="sm" className="self-start border-dashed text-muted-foreground" title={t("mission.banner.askHint")} disabled={closed || unreachable} onClick={() => actions.askMissionMaterial(mission.id, key)} data-mission-material="ask">
                <FilePlusIcon aria-hidden="true" />
                {t("mission.banner.ask")}
              </Button>
            )
          ) : null}
          {/* The choices are rows of a radio group, so a long choice wraps and reads whole; a press only marks it, and 답하기 sends it (PRD mission-human-gate B19). */}
          {options.length ? <RadioGroup value={picked == null ? "" : String(picked)} onValueChange={(value) => setPicked(Number(value))} disabled={closed} className="gap-xs" aria-label={mission.decision ?? undefined}>
            {options.map((option, index) => {
              const recommended = question?.recommend === index + 1;
              return (
                <label key={index} className={cn("flex min-w-0 cursor-pointer items-start gap-xs rounded-sm border px-md py-sm text-body transition-colors hover:bg-accent", picked === index ? "border-foreground/40 bg-accent" : recommended ? "border-mission-hold/50 bg-mission-hold/5" : "border-transparent")}>
                  <RadioGroupItem value={String(index)} className="mt-xxs shrink-0" data-mission-option={index + 1} data-mission-recommended={recommended || undefined} />
                  <span className={cn("min-w-0 flex-1 break-words", recommended && "font-semibold")}>{option}</span>
                  {recommended ? <span className="flex shrink-0 items-center gap-xxs rounded-full bg-mission-hold/20 px-sm text-caption font-semibold text-mission-hold" data-mission-recommended-tag="true"><ThumbsUpIcon aria-hidden="true" className="size-(--size-icon-sm)" />{t("mission.banner.recommended")}</span> : null}
                </label>
              );
            })}
          </RadioGroup> : null}
          <div className="flex min-w-0 flex-wrap items-center gap-xs border-t border-border pt-sm">
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
            <Button ref={writeButton} size="sm" variant="outline" disabled={closed} onClick={() => setWriting(true)} data-mission-write="true"><PencilIcon aria-hidden="true" />{t("mission.banner.write")}</Button>
          )}
          {/* A decision without choices, an old one included, is answered by writing only. */}
          {options.length ? <Button size="sm" variant="secondary" className="ml-auto" disabled={closed || picked == null} onClick={() => picked != null && choose(picked)} data-mission-answer-send="true">{t("mission.banner.answer")}</Button> : null}
          </div>
        </div>
      )}
      {view.reason ? <p className={cn("min-w-0 text-caption", view.refused ? "text-mission-failed" : "text-muted-foreground")} data-mission-banner-reason={view.reason.kind === "failed" ? view.reason.failure : view.reason.kind}>{reason}</p> : null}
    </div>
  );
}


/**
 * The Views linked to the decision, until it is answered or cleared (PRD
 * mission-human-gate B20): each a row with its kind's mark, its name, the
 * eye while the operator has not looked at it, a check once they have,
 * `닫힘` once it closed, and who linked it. Pressing an open one brings it
 * forward; pressing a closed one opens it again from what its link kept.
 */
function DecisionViews({ views, onOpen }: { views: DecisionView[]; onOpen: (view: DecisionView) => void }) {
  const { t } = useInterfaceTranslation();
  const [all, setAll] = useState(false);
  const shown = all ? views : views.slice(0, DECISION_VIEWS_SHOWN);
  const more = views.length - shown.length;
  return (
    <div className="flex min-w-0 flex-col" data-mission-banner-views={views.length}>
      <span className="text-caption text-muted-foreground">{t("mission.banner.looks", { count: views.length })}</span>
      <ul className="flex min-w-0 flex-col">
        {shown.map((view) => (
          <li key={`${view.device_id}:${view.checkout_path}:${view.display_id}`} className="min-w-0">
            <RowButton className="gap-xs px-xxs py-xxs text-body" title={view.label} onClick={() => onOpen(view)} data-mission-banner-view={view.display_id} data-mission-banner-view-by={view.by} data-mission-banner-view-state={view.closed ? "closed" : view.unseen ? "unseen" : "seen"}>
              <ViewMark view={view} />
              <span className={cn("min-w-0 flex-1 truncate", view.closed && "text-muted-foreground")}>{view.label}</span>
              {view.closed ? (
                <span className="shrink-0 text-caption text-muted-foreground">{t("mission.banner.closed")}</span>
              ) : view.unseen ? (
                <StatusMark mark="look" label={t("agents.mark.look")} />
              ) : (
                <CheckIcon role="img" aria-label={t("mission.banner.seen")} className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
              )}
              <span className="shrink-0 text-caption text-muted-foreground">{t(`mission.banner.by.${view.by}`)}</span>
              <span className="ml-xs shrink-0 text-caption font-semibold text-subtle-foreground">{t(view.closed ? "mission.banner.reopen" : "mission.banner.open")}</span>
            </RowButton>
          </li>
        ))}
        {more > 0 ? (
          <li>
            <Button variant="text" size="inline" className="px-xxs text-caption text-muted-foreground" onClick={() => setAll(true)} data-mission-banner-views-more={more}>
              {t("mission.banner.more", { count: more })}
            </Button>
          </li>
        ) : null}
      </ul>
    </div>
  );
}

/** A View's kind as its tab draws it: a page's globe, a diff's mark, a file's type. */
function ViewMark({ view }: { view: DecisionView }) {
  if (view.kind === "browser") return <GlobeIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />;
  if (view.kind === "diff") return <FileDiffIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />;
  return <FileMark icon={fileIcon(view.label)} className="size-(--size-icon-sm)" />;
}
