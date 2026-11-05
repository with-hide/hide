// One mission's page on the phone (PRD mobile-companion-ui B33-B36, B45,
// D-14, doop final board F11): its title, its Conductor and the way to that
// conversation, a cell per step, the decision the Conductor waits on with
// its choices and a written answer, what is happening now (a step that waits
// on someone, a step at work, each with its pull request and what it follows)
// and the steps that finished, folded. Which steps wait and the answer's
// state are the shared rules (`missionSteps`, `missionAnswer.ts`); this draws
// them.

import { ArrowLeftIcon, CheckIcon, ChevronRightIcon, CircleAlertIcon, Loader2Icon } from "lucide-react";
import { useEffect, type ReactNode } from "react";
import { agentProductName } from "../agentAdapters";
import { Elapsed } from "../components/elapsed";
import { ProgressCells } from "../components/mission-tone";
import { StatusMark } from "../components/status-mark";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { RowButton } from "../components/ui/row-button";
import { useInterfaceTranslation } from "../i18n/translator";
import { cn } from "../lib/utils";
import { ANSWER_LIMIT, PRESSED_MS, answerView, pressedShown, reasonKey, writtenAnswer } from "../missionAnswer";
import { STATE_PILL } from "../missionList";
import { attentionReasonText, kindWord, missionCells, missionOpen, nodeMark, stageWord } from "../missions";
import { answerMission, openDetail, settleMissionAnswer } from "./connection";
import { OfflineText } from "./OfflineText";
import { missionSteps, progress } from "./missionModel";
import { noticeText, type PhoneMission, type PhoneMissionNode } from "./protocol";
import { answerSentFor, closeMission, missionOf, setMissionDraft, setMissionFold, usePhone } from "./store";

export function MissionDetail() {
  const mission = usePhone((s) => missionOf(s.missions, s.missionOpen));
  const connected = usePhone((s) => s.connected);
  const unreachable = usePhone((s) => s.unreachable);
  // A mission that left the record while its page was open is gone with it; the store closes the page on the next frame.
  if (!mission) return null;
  return <Page mission={mission} dim={unreachable && !connected} connected={connected} />;
}

function Page({ mission, dim, connected }: { mission: PhoneMission; dim: boolean; connected: boolean }) {
  const { t } = useInterfaceTranslation();
  const open = missionOpen(mission);
  const { done, total } = progress(mission);
  const steps = missionSteps(mission);
  const foldKey = `done:${mission.id}`;
  const doneOpen = usePhone((s) => s.missionFolds[foldKey] === true);
  const stateWord = open ? (mission.conductor.present ? t("mobile.mission.conductor") : t("mission.noConductor")) : t(STATE_PILL[mission.status]);
  const conductor = mission.conductor.agent;
  const asking = open && mission.decision !== null;
  return (
    <main className="flex h-full flex-col" data-phone-mission-page={mission.id}>
      <header className="phone-safe-top shrink-0 border-b border-border bg-background px-lg pb-md">
        <div className="flex items-center justify-between gap-md pt-md">
          <Button variant="text" size="inline" className="min-h-(--size-touch-target) gap-xs text-body" data-phone-back="missions" onClick={closeMission}>
            <ArrowLeftIcon aria-hidden="true" className="size-(--size-icon)" />
            {t("mobile.tab.missions")}
          </Button>
          {!mission.stale ? <Elapsed since={mission.started_at_unix_ms ?? mission.created_at_unix_ms} className="font-mono text-body text-muted-foreground" /> : null}
        </div>
        <h1 className="break-keep text-headline font-semibold text-foreground wrap-anywhere" data-phone-mission-title="true">{mission.title}</h1>
        <div className="mt-xs flex min-w-0 items-center justify-between gap-md">
          <p className="flex min-w-0 items-center gap-xs text-subhead text-subtle-foreground" data-phone-mission-conductor={mission.conductor.present ? "present" : "none"}>
            {open && mission.conductor.present ? <Loader2Icon aria-hidden="true" className="size-(--size-icon) shrink-0 text-mission-working" /> : null}
            <span className="min-w-0 break-keep wrap-anywhere">{stateWord} · {t("mobile.mission.progress", { done, total })}</span>
          </p>
          {conductor ? (
            <Button variant="text" size="inline" className="min-h-(--size-touch-target) shrink-0 gap-0 text-subhead" data-phone-mission-chat="true" onClick={() => openDetail(conductor)}>
              {t("mobile.mission.chat")}
              <ChevronRightIcon aria-hidden="true" className="size-(--size-icon)" />
            </Button>
          ) : null}
        </div>
        {mission.nodes.length > 0 ? <ProgressCells tones={missionCells(mission)} size="head" className="mt-sm w-full" /> : null}
      </header>
      {dim ? (
        <div role="status" className="flex items-center gap-sm px-lg py-sm text-body text-subtle-foreground" data-phone-unreachable="true">
          <Loader2Icon aria-hidden="true" className="size-(--size-icon-lg) shrink-0 animate-spin" />
          <span><OfflineText /></span>
        </div>
      ) : null}
      <div className={cn("min-h-0 flex-1 overflow-auto overscroll-contain px-lg pb-lg", dim && "opacity-(--opacity-dimmed)")} aria-busy={dim}>
        {asking ? <DecisionCard mission={mission} connected={connected} /> : null}
        {steps.hold.length + steps.working.length > 0 ? (
          <section className="pt-lg" data-phone-mission-now="true">
            <h2 className="flex items-baseline justify-between gap-md pb-xs text-body text-muted-foreground">
              <span>{t("mobile.mission.now")}</span>
              <span className="text-subtle-foreground">
                {[steps.working.length > 0 ? t("mobile.mission.working", { count: steps.working.length }) : null, steps.hold.length > 0 ? t("mobile.mission.hold", { count: steps.hold.length }) : null].filter(Boolean).join(" · ")}
              </span>
            </h2>
            <ul className="divide-y divide-border border-b border-border">
              {steps.hold.map((node) => <Step key={node.key} mission={mission} node={node} kind="hold" />)}
              {steps.working.map((node) => <Step key={node.key} mission={mission} node={node} kind="working" />)}
            </ul>
          </section>
        ) : null}
        {steps.done.length > 0 ? (
          <section className="pt-lg" data-phone-mission-done={doneOpen ? "open" : "folded"}>
            <h2 className="flex items-center justify-between gap-md pb-xs text-body text-muted-foreground">
              <span>{t("mobile.mission.finished", { count: steps.done.length })}</span>
              <Button variant="text" size="inline" aria-expanded={doneOpen} className="min-h-(--size-touch-target)" data-phone-fold={foldKey} onClick={() => setMissionFold(foldKey, !doneOpen)}>
                {doneOpen ? t("missionTool.fewer") : t("mobile.mission.unfold")}
              </Button>
            </h2>
            {doneOpen ? (
              <ul className="divide-y divide-border border-b border-border">
                {steps.done.map((node) => <Step key={node.key} mission={mission} node={node} kind="done" />)}
              </ul>
            ) : null}
          </section>
        ) : null}
      </div>
      {asking ? <WrittenAnswer mission={mission} connected={connected} /> : null}
    </main>
  );
}

/**
 * The decision (B34, B35): the question, why it matters, the Conductor's
 * choices with its recommendation, and where the answer is once sent; a
 * refusal's reason in the desktop's words stands under it, and what was
 * typed stays.
 */
function DecisionCard({ mission, connected }: { mission: PhoneMission; connected: boolean }) {
  const { t } = useInterfaceTranslation();
  const sent = usePhone((s) => answerSentFor(s.missionAnswer, mission.id));
  const error = usePhone((s) => (s.missionAnswerError?.mission === mission.id ? s.missionAnswerError.notice : null));
  const pressed = sent?.pressed ?? null;
  const view = answerView(mission, mission.device_label, pressed);
  const question = mission.question;
  const stopped = question?.node ? (mission.nodes.find((node) => node.key === question.node) ?? null) : null;
  // What was pressed shows until the core answers for it, refuses it, or asks something else (B26).
  const shown = pressed === null || pressedShown(mission, pressed, view);
  useEffect(() => {
    if (!pressed) return;
    if (!shown) return settleMissionAnswer();
    const timer = window.setTimeout(settleMissionAnswer, PRESSED_MS);
    return () => window.clearTimeout(timer);
  }, [pressed, shown]);
  const resend = () => {
    if (view.sent) answerMission(mission, { resend: true }, view.sent.text);
  };
  return (
    <section
      aria-label={mission.decision ?? undefined}
      className="mt-lg flex min-w-0 flex-col gap-sm rounded-lg border border-mission-hold/40 bg-card px-md py-md"
      data-phone-decision={view.sent?.state ?? (view.closed ? "closed" : "open")}
    >
      <p className="flex min-w-0 items-center gap-sm text-body font-semibold text-mission-hold">
        <StatusMark mark="turn" />
        <span className="min-w-0 break-keep wrap-anywhere">{stopped ? t("mobile.mission.decisionAt", { node: stopped.title }) : t("mobile.mission.decision")}</span>
      </p>
      <p className="break-keep text-title leading-normal text-foreground wrap-anywhere" data-phone-question="true">{mission.decision}</p>
      {question?.why ? <p className="break-keep text-body text-muted-foreground wrap-anywhere">{question.why}</p> : null}
      {view.sent?.state === "unconfirmed" ? (
        <div className="flex min-w-0 flex-col gap-xs" data-phone-answer-sent="unconfirmed">
          <p className="flex min-w-0 items-start gap-xs text-body">
            <CircleAlertIcon aria-hidden="true" className="mt-xxs size-(--size-icon) shrink-0 text-mission-hold" />
            <span className="min-w-0 break-keep font-semibold wrap-anywhere">{view.sent.text}</span>
          </p>
          <p className="text-body text-mission-hold">{t("mission.banner.unconfirmed")}</p>
          <Button variant="secondary" className="min-h-(--size-touch-target) self-start" disabled={view.unreachable || !connected || sent !== null} data-phone-resend="true" onClick={resend}>
            {t("mission.banner.resend")}
          </Button>
        </div>
      ) : view.sent?.state === "waiting" && mission.answer_failed === "session_unreadable" ? (
        // The answer waits on a Conductor session Hide cannot read: it is not delivered until a read lands.
        <p className="flex min-w-0 items-start gap-xs text-body" data-phone-answer-sent="waiting">
          <CircleAlertIcon aria-hidden="true" className="mt-xxs size-(--size-icon) shrink-0 text-mission-failed" />
          <span className="min-w-0 break-keep wrap-anywhere"><span className="font-semibold">{view.sent.text}</span> <span className="text-mission-failed">· {t("mission.banner.failed.session_unreadable")}</span></span>
        </p>
      ) : view.sent ? (
        <p className="flex min-w-0 items-start gap-xs text-body" data-phone-answer-sent={view.sent.state}>
          <CheckIcon aria-hidden="true" className="mt-xxs size-(--size-icon) shrink-0 text-mission-done" />
          <span className="min-w-0 break-keep wrap-anywhere"><span className="font-semibold text-mission-done">{view.sent.text}</span> <span className="text-muted-foreground">· {t(view.sent.state === "waiting" ? "mission.banner.waitingTurn" : "mission.banner.reading")}</span></span>
        </p>
      ) : (
        <div className="flex min-w-0 flex-wrap gap-sm">
          {view.options.map((option, index) => {
            const recommended = view.recommend === index + 1;
            return (
              <Button
                key={index}
                variant="secondary"
                disabled={view.closed || !connected}
                className={cn("h-auto min-h-(--size-touch-target) max-w-full whitespace-normal break-keep px-md py-xs text-body wrap-anywhere", recommended && "font-semibold")}
                data-phone-option={index + 1}
                data-phone-recommended={recommended || undefined}
                onClick={() => answerMission(mission, { option: index + 1 }, option)}
              >
                <span className="min-w-0">{option}</span>
                {recommended ? <span className="shrink-0 rounded-xs bg-background px-xxs text-micro text-foreground">{t("mission.banner.recommended")}</span> : null}
              </Button>
            );
          })}
        </div>
      )}
      {view.reason ? (
        <p
          className={cn("break-keep text-body wrap-anywhere", !view.sent && mission.answer_failed && view.reason.kind === "failed" ? "text-mission-failed" : "text-muted-foreground")}
          data-phone-answer-reason={view.reason.kind === "failed" ? view.reason.failure : view.reason.kind}
        >
          {t(reasonKey(view.reason), { device: mission.device_label ?? "" })}
        </p>
      ) : null}
      {error ? <p role="alert" className="break-keep text-body text-destructive wrap-anywhere" data-phone-answer-error="true">{noticeText(t, error)}</p> : null}
    </section>
  );
}

/** The written answer (B34): one line at the foot of the page, kept when the answer is refused (B35); nothing is sent while the Mac is out of reach (B45). */
function WrittenAnswer({ mission, connected }: { mission: PhoneMission; connected: boolean }) {
  const { t } = useInterfaceTranslation();
  const draft = usePhone((s) => s.missionDrafts[mission.id] ?? "");
  const sent = usePhone((s) => answerSentFor(s.missionAnswer, mission.id));
  const pressed = sent?.pressed ?? null;
  const view = answerView(mission, mission.device_label, pressed);
  const blocked = view.closed || !connected || sent !== null;
  const text = writtenAnswer(draft);
  const left = ANSWER_LIMIT - draft.length;
  return (
    <footer className="phone-safe-bottom shrink-0 border-t border-border bg-background px-lg pt-sm">
      <form
        className="flex gap-sm pb-md"
        onSubmit={(event) => {
          event.preventDefault();
          if (text && !blocked) answerMission(mission, { text }, text);
        }}
      >
        <Input
          value={draft}
          maxLength={ANSWER_LIMIT}
          onChange={(event) => setMissionDraft(mission.id, event.target.value)}
          placeholder={t("mobile.mission.replyPlaceholder")}
          aria-label={t("mission.banner.input")}
          disabled={view.closed || !connected}
          enterKeyHint="send"
          autoComplete="off"
          autoCorrect="off"
          data-phone-answer-input="true"
          className="h-(--size-touch-target) min-w-0 flex-1 rounded-lg border border-border bg-input px-md text-title text-foreground outline-none placeholder:text-muted-foreground focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-(--opacity-disabled)"
        />
        <Button variant="secondary" type="submit" disabled={blocked || text.length === 0} className="h-(--size-touch-target) rounded-md px-md text-body font-medium" data-phone-answer-send="true">
          {t("mobile.send")}
        </Button>
      </form>
      {left <= 100 ? <p className="-mt-sm pb-md text-body text-muted-foreground" data-phone-answer-left={left}>{t("mission.banner.left", { count: left })}</p> : null}
    </footer>
  );
}

/**
 * One step (B36): its mark, title, and under it its pull request, what it
 * waits on or follows and its worker; pressing it opens the worker's detail
 * when the phone's list holds that agent.
 */
function Step({ mission, node, kind }: { mission: PhoneMission; node: PhoneMissionNode; kind: "hold" | "working" | "done" }) {
  const { t } = useInterfaceTranslation();
  const mark = kind === "hold" ? "turn" : nodeMark(node, null);
  const detail = stepLine(mission, node, kind, t);
  const worker = node.worker_agent ? agentProductName(node.worker_agent) : null;
  const body: ReactNode = (
    <>
      <span className="flex w-full min-w-0 items-start gap-sm">
        <span className="flex h-(--size-touch-target) shrink-0 items-center"><StatusMark mark={mark} /></span>
        <span className="flex min-w-0 flex-1 flex-col justify-center gap-xs py-sm">
          <span className={cn("break-keep text-title wrap-anywhere", kind === "done" ? "text-subtle-foreground" : "text-foreground")}>{node.title}</span>
          <span className="flex min-w-0 flex-wrap items-center gap-sm text-body text-muted-foreground">
            {node.pr != null ? <span className="shrink-0 rounded-xs bg-secondary px-xs font-mono text-caption text-subtle-foreground" data-phone-step-pr={node.pr}>#{node.pr}</span> : null}
            {detail ? <span className="min-w-0 break-keep wrap-anywhere" data-phone-step-line={kind}>{detail}</span> : null}
            {worker ? <span className="shrink-0">· {worker}</span> : null}
          </span>
        </span>
        {node.agent ? <ChevronRightIcon aria-hidden="true" className="mt-md size-(--size-icon) shrink-0 text-muted-foreground" /> : null}
      </span>
    </>
  );
  const agent = node.agent;
  return (
    <li data-phone-step={node.key} data-phone-step-kind={kind}>
      {agent ? (
        <RowButton aria-label={`${node.title}${worker ? `, ${worker}` : ""}`} className="min-h-(--size-touch-target) flex-col items-stretch rounded-none px-none py-none hover:bg-transparent active:bg-accent" data-phone-step-open="true" onClick={() => openDetail(agent)}>
          {body}
        </RowButton>
      ) : (
        body
      )}
    </li>
  );
}

/** What a step's second line says besides its pull request (B36): why it waits, what it follows, where its work stands, or how it ended. */
function stepLine(mission: PhoneMission, node: PhoneMissionNode, kind: "hold" | "working" | "done", t: ReturnType<typeof useInterfaceTranslation>["t"]): string | null {
  if (kind === "hold") {
    if (node.attention && mission.status !== "cancelled") return attentionReasonText(node.attention.reason, node.attention.pr, t);
    if (node.escalation) return node.escalation.line ?? t(`mission.turn.cause.${node.escalation.cause}`);
    return t("mobile.mission.waitsDecision");
  }
  if (kind === "done") return node.pr_state ? t(`agentSessions.pr.${node.pr_state}`) : t(kindWord(node.kind));
  return node.base_pr != null ? t("mobile.mission.after", { pr: String(node.base_pr) }) : t(stageWord(node));
}
