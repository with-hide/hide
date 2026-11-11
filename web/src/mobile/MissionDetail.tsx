// One mission's page on the phone (PRD mobile-companion-ui B33-B36, B45,
// D-14, doop final board F11): its title, its Conductor and the way to that
// conversation, a cell per step, the decision the Conductor waits on with
// its choices and a written answer, the safety net over the raises its
// Conductor has not settled, what is happening now (a step that waits
// on someone, a step at work, each with its pull request and what it follows)
// the lines the Conductor skipped on its own and the steps that finished,
// both folded. Which steps wait and the answer's
// state are the shared rules (`missionSteps`, `missionAnswer.ts`); this draws
// them.

import { ArrowLeftIcon, CheckIcon, ChevronRightIcon, CircleAlertIcon, FilePlusIcon, GitPullRequestIcon, Loader2Icon, ThumbsUpIcon } from "lucide-react";
import { useState, type ReactNode } from "react";
import { agentProductName } from "../agentAdapters";
import { Elapsed, useElapsed, useMinuteClock } from "../components/elapsed";
import { GatePlanList, GateStrip } from "../components/mission-gate";
import { ProgressCells } from "../components/mission-tone";
import { StatusMark } from "../components/status-mark";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { RadioGroup, RadioGroupItem } from "../components/ui/radio-group";
import { RowButton } from "../components/ui/row-button";
import { useInterfaceTranslation } from "../i18n/translator";
import { cn } from "../lib/utils";
import { ANSWER_LIMIT, answerView, reasonKey, usePressedAnswer, writtenAnswer } from "../missionAnswer";
import { STATE_PILL } from "../missionList";
import { PR_LOOK } from "../prMark";
import { DECISION_VIEWS_SHOWN, attentionReasonText, gatesAhead, gateWaitedLong, escalationWhy, holdReason, kindWord, missionCells, missionOpen, nodeMark, openAttention, skipWords, stageWord } from "../missions";
import { answerMission, askMissionMaterial, openDetail, reopenMissionView, settleMissionAnswer } from "./connection";
import { OfflineText } from "./OfflineText";
import { missionSteps, progress, workerOf } from "./missionModel";
import type { MissionHeldNode, MissionSafetyNet, MissionSkip, PrState } from "../snapshot";
import { noticeText, type AgentKey, type PhoneDecisionView, type PhoneMission, type PhoneMissionNode } from "./protocol";
import { agentOf, answerSentFor, closeMission, missionOf, setMissionDraft, setMissionFold, usePhone } from "./store";

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
  const ahead = mission.status === "running" ? gatesAhead(mission).filter((node) => node.status === "planned") : [];
  const foldKey = `done:${mission.id}`;
  const doneOpen = usePhone((s) => s.missionFolds[foldKey] === true);
  const stateWord = open ? (mission.conductor.present ? t("mobile.mission.conductor") : t("mission.noConductor")) : t(STATE_PILL[mission.status]);
  const conductor = usePhone((s) => (mission.conductor.agent && agentOf(s.groups, mission.conductor.agent) ? mission.conductor.agent : null));
  const asking = open && mission.decision !== null;
  // A finished mission whose steps stayed on the Mac has no progress to count.
  const stepsOmitted = mission.nodes_omitted ?? 0;
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
            <span className="min-w-0 break-keep wrap-anywhere">{stepsOmitted > 0 ? stateWord : `${stateWord} · ${t("mobile.mission.progress", { done, total })}`}</span>
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
        {open && mission.safety_net ? <SafetyNet mission={mission} net={mission.safety_net} conductor={conductor} /> : null}
        {/* Where the operator will look, over a plan that waits for approval, as the desktop's map lists it (PRD mission-human-gate B1, B13). */}
        <GatePlanList nodes={mission.status === "planning" ? gatesAhead(mission) : []} className="pt-lg" />
        {stepsOmitted > 0 ? (
          <p className="pt-lg text-body text-muted-foreground" data-phone-mission-steps-omitted={stepsOmitted}>{t("mobile.mission.stepsOmitted", { count: stepsOmitted })}</p>
        ) : null}
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
        {ahead.length > 0 ? (
          <section className="pt-lg" data-phone-mission-ahead="true">
            <h2 className="pb-xs text-body text-muted-foreground">{t("mission.gate.planTitle", { count: ahead.length })}</h2>
            <ul className="divide-y divide-border border-b border-border">
              {ahead.map((node) => <Step key={node.key} mission={mission} node={node} kind="ahead" />)}
            </ul>
          </section>
        ) : null}
        <Decided mission={mission} />
        {steps.done.length > 0 ? (
          <section className="pt-lg" data-phone-mission-done={doneOpen ? "open" : "folded"}>
            <h2 className="flex items-center justify-between gap-md pb-xs text-body text-muted-foreground">
              <span>{t("mobile.mission.finished", { count: steps.done.length })}</span>
              <Button variant="text" size="inline" aria-expanded={doneOpen} className="min-h-(--size-touch-target)" data-phone-fold={foldKey} onClick={() => setMissionFold(foldKey, !doneOpen)}>
                {doneOpen ? t("missionScreen.fewer") : t("mobile.mission.unfold")}
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
 * The mission's one card for the raises its Conductor has not settled, as
 * the desktop's 내 차례 draws it (the principle of 2026-11-11, hide-private
 * docs/missions.md): how many stand and since when, or that no Conductor
 * holds the mission, a row per held step with its reason and age, and the
 * Conductor's conversation while it holds the mission. The phone starts no
 * Conductor: 지휘 다시 열기 is the Mac's.
 */
function SafetyNet({ mission, net, conductor }: { mission: PhoneMission; net: MissionSafetyNet; conductor: AgentKey | null }) {
  const { t } = useInterfaceTranslation();
  const age = useElapsed(mission.stale ? null : net.since_unix_ms);
  const count = net.nodes.length;
  const head = !net.conductor_present ? t("mission.net.noConductor", { count }) : age ? t("mission.net.title", { count, age }) : t("mission.net.line", { count });
  return (
    <section className="mt-lg flex min-w-0 flex-col gap-sm rounded-lg border border-mission-hold/40 bg-card px-md py-md" data-phone-safety-net={net.conductor_present ? "present" : "absent"}>
      <p className="flex min-w-0 items-center gap-sm text-body font-semibold text-mission-hold">
        <StatusMark mark="turn" />
        <span className="min-w-0 break-keep wrap-anywhere" data-phone-safety-net-head="true">{head}</span>
      </p>
      {net.conductor_present && age ? <p className="break-keep text-body text-muted-foreground wrap-anywhere">{t("mission.net.unsettled")}</p> : null}
      <ul className="flex min-w-0 flex-col gap-xs">
        {net.nodes.map((held) => <HeldStep key={held.key} mission={mission} held={held} />)}
      </ul>
      {net.conductor_present && conductor ? (
        <Button variant="secondary" className="min-h-(--size-touch-target) self-start" data-phone-safety-net-chat="true" onClick={() => openDetail(conductor)}>
          {t("mobile.mission.chat")}
        </Button>
      ) : null}
    </section>
  );
}

function HeldStep({ mission, held }: { mission: PhoneMission; held: MissionHeldNode }) {
  const { t } = useInterfaceTranslation();
  const age = useElapsed(mission.stale ? null : held.since_unix_ms);
  return (
    <li className="flex min-w-0 flex-col" data-phone-held-step={held.key}>
      <span className="break-keep text-body text-foreground wrap-anywhere">{held.title}</span>
      <span className="break-keep text-caption text-muted-foreground wrap-anywhere">
        {attentionReasonText(held.reason, held.pr, t)}
        {age ? ` · ${t("mission.net.age", { age })}` : null}
      </span>
    </li>
  );
}

/** `Conductor가 정한 것 N`, folded: the lines the Conductor skipped on its own, each the step, `<line> 넘김`, why, who and how long ago. */
function Decided({ mission }: { mission: PhoneMission }) {
  const { t } = useInterfaceTranslation();
  const key = `decided:${mission.id}`;
  const unfolded = usePhone((s) => s.missionFolds[key] === true);
  const skips = mission.skipped_alone ?? [];
  if (skips.length === 0) return null;
  return (
    <section className="pt-lg" data-phone-mission-decided={unfolded ? "open" : "folded"}>
      <h2 className="flex items-center justify-between gap-md pb-xs text-body text-muted-foreground">
        <span>{t("mission.decided", { count: skips.length })}</span>
        <Button variant="text" size="inline" aria-expanded={unfolded} className="min-h-(--size-touch-target)" data-phone-fold={key} onClick={() => setMissionFold(key, !unfolded)}>
          {unfolded ? t("missionScreen.fewer") : t("mobile.mission.unfold")}
        </Button>
      </h2>
      {unfolded ? (
        <ul className="divide-y divide-border border-b border-border">
          {skips.map((skip) => <SkipStep key={`${skip.node}:${skip.line}`} skip={skip} />)}
        </ul>
      ) : null}
    </section>
  );
}

function SkipStep({ skip }: { skip: MissionSkip }) {
  const { t } = useInterfaceTranslation();
  const ago = useElapsed(skip.at_unix_ms);
  return (
    <li className="flex min-w-0 flex-col gap-xxs py-sm" data-phone-skip={`${skip.node}:${skip.line}`}>
      <span className="break-keep text-body text-foreground wrap-anywhere">{skip.title}</span>
      <span className="break-keep text-caption text-muted-foreground wrap-anywhere">{skipWords(skip, t)} · {skip.reason}</span>
      <span className="text-caption text-subtle-foreground">{[skip.by, ago].filter(Boolean).join(" · ")}</span>
    </li>
  );
}

/**
 * The decision (B34, B35): the question, why it matters, the Conductor's
 * choices with its recommendation, and where the answer is once sent; a
 * refusal's reason in the desktop's words stands under it, and what was
 * typed stays. As on the desktop's card, a choice is a radio row a press
 * only marks, and `답하기` sends it (PRD mission-human-gate D-19, B19).
 */
function DecisionCard({ mission, connected }: { mission: PhoneMission; connected: boolean }) {
  const { t } = useInterfaceTranslation();
  const sent = usePhone((s) => answerSentFor(s.missionAnswer, mission.id));
  const error = usePhone((s) => (s.missionAnswerError?.mission === mission.id ? s.missionAnswerError.notice : null));
  const pressed = sent?.pressed ?? null;
  const view = answerView(mission, mission.device_label, pressed);
  const question = mission.question;
  const stopped = question?.node ? (mission.nodes.find((node) => node.key === question.node) ?? null) : null;
  // The choice marked on this question; a new question starts unmarked.
  const [picked, setPicked] = useState<{ key: number; index: number } | null>(null);
  const marked = picked?.key === view.key ? picked.index : null;
  // What was pressed shows until the core answers for it, refuses it, or asks something else (B26).
  usePressedAnswer(mission, pressed, view, settleMissionAnswer);
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
      {mission.decision_views?.length ? <DecisionViews mission={mission} views={mission.decision_views} connected={connected} /> : null}
      {view.sent?.line === "unconfirmed" ? (
        <div className="flex min-w-0 flex-col gap-xs" data-phone-answer-sent="unconfirmed">
          <p className="flex min-w-0 items-start gap-xs text-body">
            <CircleAlertIcon aria-hidden="true" className="mt-xxs size-(--size-icon) shrink-0 text-mission-hold" />
            <span className="min-w-0 break-keep font-semibold wrap-anywhere">{view.sent.text}</span>
          </p>
          <p className="text-body text-mission-hold">{t(view.sent.word)}</p>
          <Button variant="secondary" className="min-h-(--size-touch-target) self-start" disabled={view.unreachable || !connected || sent !== null} data-phone-resend="true" onClick={resend}>
            {t("mission.banner.resend")}
          </Button>
        </div>
      ) : view.sent?.line === "unreadable" ? (
        // The answer waits on a Conductor session Hide cannot read: it is not delivered until a read lands.
        <p className="flex min-w-0 items-start gap-xs text-body" data-phone-answer-sent="waiting">
          <CircleAlertIcon aria-hidden="true" className="mt-xxs size-(--size-icon) shrink-0 text-mission-failed" />
          <span className="min-w-0 break-keep wrap-anywhere"><span className="font-semibold">{view.sent.text}</span> <span className="text-mission-failed">· {t(view.sent.word)}</span></span>
        </p>
      ) : view.sent ? (
        <p className="flex min-w-0 items-start gap-xs text-body" data-phone-answer-sent={view.sent.state}>
          <CheckIcon aria-hidden="true" className="mt-xxs size-(--size-icon) shrink-0 text-mission-done" />
          <span className="min-w-0 break-keep wrap-anywhere"><span className="font-semibold text-mission-done">{view.sent.text}</span> <span className="text-muted-foreground">· {t(view.sent.word)}</span></span>
        </p>
      ) : (
        <div className="flex min-w-0 flex-col gap-sm">
          {/* With nothing linked to look at, asking for material is a row of its own above the choices, as on the desktop's card; it never holds the answer back. */}
          {!mission.decision_views?.length && question ? (
            question.material_asked ? (
              <p className="flex min-w-0 items-center gap-xs text-body text-muted-foreground" data-phone-material="asked">
                <CheckIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-mission-done" />
                {t("mission.banner.asked")}
              </p>
            ) : (
              <div className="flex min-w-0 flex-col gap-xxs">
                <Button
                  variant="outline"
                  className="min-h-(--size-touch-target) self-start border-dashed text-body text-muted-foreground"
                  disabled={view.closed || view.unreachable || !connected}
                  data-phone-material="ask"
                  onClick={() => askMissionMaterial(mission)}
                >
                  <FilePlusIcon aria-hidden="true" />
                  {t("mission.banner.ask")}
                </Button>
                {/* The phone has no hover, so why to ask is read out, not hinted. */}
                <span className="break-keep text-caption text-muted-foreground wrap-anywhere">{t("mission.banner.askHint")}</span>
              </div>
            )
          ) : null}
          <RadioGroup
            value={marked == null ? "" : String(marked)}
            onValueChange={(value) => setPicked({ key: view.key, index: Number(value) })}
            disabled={view.closed || !connected}
            className="gap-xs"
            aria-label={mission.decision ?? undefined}
          >
            {view.options.map((option, index) => {
              const recommended = view.recommend === index + 1;
              return (
                <label
                  key={index}
                  className={cn(
                    "flex min-h-(--size-touch-target) min-w-0 cursor-pointer items-start gap-sm rounded-md border px-md py-sm text-body",
                    marked === index ? "border-foreground/40 bg-accent" : recommended ? "border-mission-hold/50 bg-mission-hold/5" : "border-border",
                  )}
                >
                  <RadioGroupItem value={String(index)} className="mt-xxs shrink-0" data-phone-option={index + 1} data-phone-recommended={recommended || undefined} />
                  <span className={cn("min-w-0 flex-1 break-keep wrap-anywhere", recommended && "font-semibold")}>{option}</span>
                  {recommended ? (
                    <span className="flex shrink-0 items-center gap-xxs rounded-full bg-mission-hold/20 px-sm text-caption font-semibold text-mission-hold" data-phone-recommended-tag="true">
                      <ThumbsUpIcon aria-hidden="true" className="size-(--size-icon-sm)" />
                      {t("mission.banner.recommended")}
                    </span>
                  ) : null}
                </label>
              );
            })}
          </RadioGroup>
          {view.options.length ? (
            <Button
              variant="secondary"
              className="min-h-(--size-touch-target) self-end"
              disabled={view.closed || !connected || marked == null}
              data-phone-choice-send="true"
              onClick={() => marked != null && answerMission(mission, { option: marked + 1 }, view.options[marked]!)}
            >
              {t("mission.banner.answer")}
            </Button>
          ) : null}
        </div>
      )}
      {view.reason ? (
        <p
          className={cn("break-keep text-body wrap-anywhere", view.refused ? "text-mission-failed" : "text-muted-foreground")}
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
function Step({ mission, node, kind }: { mission: PhoneMission; node: PhoneMissionNode; kind: "hold" | "working" | "done" | "ahead" }) {
  const { t } = useInterfaceTranslation();
  // A step held by a raise its Conductor settles waits; only what the operator settles is their turn.
  const mark = kind === "hold" ? (openAttention(mission, node)?.to === "conductor" ? "waiting" : "turn") : nodeMark(node, null);
  const detail = stepLine(mission, node, kind, t);
  // The step's review gate, as the desktop's card draws it; reached past 24 hours it says so in grey (PRD mission-human-gate B13, B22).
  const minute = useMinuteClock();
  const long = gateWaitedLong(node.gate, minute);
  const worker = node.worker_agent ? agentProductName(node.worker_agent) : null;
  // A row hided left out of a long list is not one the phone can open.
  const agent = usePhone((s) => (node.agent && agentOf(s.groups, node.agent) ? node.agent : null));
  const body: ReactNode = (
    <>
      <span className="flex w-full min-w-0 items-start gap-sm">
        <span className="flex h-(--size-touch-target) shrink-0 items-center"><StatusMark mark={mark} /></span>
        <span className="flex min-w-0 flex-1 flex-col justify-center gap-xs py-sm">
          <span className={cn("break-keep text-title wrap-anywhere", kind === "done" ? "text-subtle-foreground" : "text-foreground")}>{node.title}</span>
          <span className="flex min-w-0 flex-wrap items-center gap-sm text-body text-muted-foreground">
            {node.pr != null ? <StepPr number={node.pr} state={node.pr_state} /> : null}
            {detail ? <span className="min-w-0 break-keep wrap-anywhere" data-phone-step-line={kind}>{detail}</span> : null}
            {worker ? <span className="shrink-0">· {worker}</span> : null}
          </span>
          {node.gate ? (
            <span className="flex min-w-0 flex-col gap-xxs">
              <GateStrip gate={node.gate} className="text-body" />
              {long ? <span className="text-caption text-muted-foreground" data-phone-step-gate-long="true">{t("agents.mark.turn")} · {t("mission.gate.long")}</span> : null}
              {node.gate.state === "passed" && node.gate.via ? (
                <span className="min-w-0 break-keep text-caption text-muted-foreground wrap-anywhere" data-phone-step-gate-via={node.gate.via}>
                  {t(`mission.gate.via.${node.gate.via}`)}
                  {node.gate.said ? <> · “{node.gate.said}”</> : null}
                </span>
              ) : null}
            </span>
          ) : null}
        </span>
        {agent ? <ChevronRightIcon aria-hidden="true" className="mt-md size-(--size-icon) shrink-0 text-muted-foreground" /> : null}
      </span>
    </>
  );
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

/** A step's pull request: its state's icon and colour and `#number`, the desktop's PR mark; one whose state Hide never read keeps the muted icon. */
function StepPr({ number, state }: { number: number; state: PrState | null | undefined }) {
  const look = state ? PR_LOOK[state] : null;
  const Icon = look?.icon ?? GitPullRequestIcon;
  return (
    <span className="inline-flex shrink-0 items-center gap-xxs font-mono text-caption text-subtle-foreground" data-phone-step-pr={number} data-phone-step-pr-state={state ?? "unread"}>
      <Icon aria-hidden="true" className={cn("size-(--size-pr-icon) shrink-0", look?.tone ?? "text-muted-foreground")} />
      #{number}
    </span>
  );
}

/** What a step's second line says besides its pull request (B36): why it waits, what it follows, where its work stands, or how it ended. */
function stepLine(mission: PhoneMission, node: PhoneMissionNode, kind: "hold" | "working" | "done" | "ahead", t: ReturnType<typeof useInterfaceTranslation>["t"]): string | null {
  if (kind === "ahead") return t("mission.gate.when");
  if (kind === "hold") {
    const reason = holdReason(mission, node, workerOf);
    switch (reason?.kind) {
      case "attention": {
        // As the desktop's summary says it: who the raise waits on, then why.
        const why = attentionReasonText(reason.attention.reason, reason.attention.pr, t);
        return reason.attention.to === "conductor" ? `${t("mission.attention.to.conductor")} · ${why}` : why;
      }
      case "worker":
        return escalationWhy(reason.escalation, t);
      case "decision":
        return t("mobile.mission.waitsDecision");
      default:
        return null;
    }
  }
  if (kind === "done") return node.pr_state ? t(`agentSessions.pr.${node.pr_state}`) : t(kindWord(node.kind));
  return node.base_pr != null ? t("mobile.mission.after", { pr: String(node.base_pr) }) : t(stageWord(node));
}

/**
 * The Views linked to the decision, seen, unseen or closed (PRD
 * mission-human-gate B20): a row opens the 볼 것 of the agent that opened
 * it, where the View opens full screen; a closed one, marked `닫힘`, asks
 * the Mac to open it again first and then opens that 볼 것. One whose agent
 * is closed is listed without a press.
 */
function DecisionViews({ mission, views, connected }: { mission: PhoneMission; views: PhoneDecisionView[]; connected: boolean }) {
  const { t } = useInterfaceTranslation();
  const [all, setAll] = useState(false);
  const reopening = usePhone((s) => (s.missionReopen?.mission === mission.id ? s.missionReopen.view : null));
  const busy = usePhone((s) => s.missionReopen !== null);
  const shown = all ? views : views.slice(0, DECISION_VIEWS_SHOWN);
  const more = views.length - shown.length;
  return (
    <div className="flex min-w-0 flex-col" data-phone-decision-views={views.length}>
      <span className="text-body text-muted-foreground">{t("mission.banner.looks", { count: views.length })}</span>
      <ul className="flex min-w-0 flex-col divide-y divide-border">
        {shown.map((view) => {
          const pane = view.pane_id;
          const waiting = reopening === view.view;
          const words = (
            <>
              <span className="min-w-0 flex-1 break-keep text-body wrap-anywhere">{view.label}</span>
              {/* The phone has no hover, so the mark's words are read out, not hinted. */}
              {waiting ? (
                <span className="shrink-0 text-body text-muted-foreground" data-phone-decision-view-opening="true">{t("mobile.mission.reopen.opening")}</span>
              ) : view.closed ? (
                <span className="shrink-0 text-body text-muted-foreground">{t("mission.banner.closed")}</span>
              ) : view.unseen ? (
                <>
                  <StatusMark mark="look" />
                  <span className="sr-only">{t("agents.mark.look")}</span>
                </>
              ) : null}
              <span className="shrink-0 text-body text-muted-foreground">{t(`mission.banner.by.${view.by}`)}</span>
            </>
          );
          // A device on a build before the row's name (`view`) cannot be asked to open its closed row again.
          const press = !pane || (view.closed && !view.view)
            ? null
            : view.closed
              ? () => reopenMissionView(mission, view)
              : () => openDetail({ device_id: view.device_id, pane_id: pane }, "looks");
          return (
            <li key={view.view} className="min-w-0">
              {press ? (
                <RowButton
                  className="min-h-(--size-touch-target) gap-sm px-0"
                  data-phone-decision-view={view.display_id}
                  disabled={view.closed && (busy || !connected)}
                  onClick={press}
                >
                  {words}
                </RowButton>
              ) : (
                <span className="flex min-h-(--size-touch-target) min-w-0 items-center gap-sm opacity-(--opacity-dimmed)" data-phone-decision-view={view.display_id}>{words}</span>
              )}
            </li>
          );
        })}
        {more > 0 ? (
          <li>
            <Button variant="text" size="inline" className="min-h-(--size-touch-target) text-body" data-phone-decision-views-more={more} onClick={() => setAll(true)}>
              {t("mission.banner.more", { count: more })}
            </Button>
          </li>
        ) : null}
      </ul>
    </div>
  );
}
