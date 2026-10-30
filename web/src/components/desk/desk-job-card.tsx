import { ChevronRightIcon, GitBranchIcon, GlobeIcon, ImageIcon } from "lucide-react";
import type { ReactNode } from "react";
import type { DeskAsk, DeskJob, DeskMission, DeskOutput, DeskSessionRef } from "../../desk";
import { AgentMark } from "../../AgentMark";
import { fileIcon } from "../../fileIcons";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import type { MessageKey } from "../../i18n/catalogs";
import type { CellTone } from "../../missions";
import { PlanChipFace, planChipLook } from "../../PanePlanChip";
import { DeviceChip } from "../device-chip";
import { FileMark } from "../file-mark";
import { MissionChipView } from "../mission-chip";
import { PrMark } from "../pr-mark";
import { StatusMark } from "../status-mark";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { RowOverlayButton } from "../ui/row-button";

type T = ReturnType<typeof useInterfaceTranslation>["t"];

/** The verb an asking job's line starts with (D-75): every wait for the operator reads `<verb> · …`. */
const ASK_VERB: Record<DeskAsk, MessageKey> = {
  question: "desk.ask.question",
  approval: "desk.ask.approval",
  decision: "desk.ask.decision",
  plan_review: "desk.ask.plan_review",
  blocked: "desk.ask.blocked",
};

/** A job's state in its words (B21-B24, B36-B39): what the second line says when nothing more specific does. */
export function jobStateWords(t: T, job: DeskJob): string {
  switch (job.state) {
    case "queued":
      return t("desk.job.queued", { position: job.queue_position ?? 0 });
    case "starting":
      return t("desk.job.starting");
    case "working":
      return t("desk.job.working");
    case "asking":
      return t(ASK_VERB[job.ask ?? "question"]);
    case "done":
      return t("desk.job.done");
    case "sleeping":
      return t("desk.job.sleeping");
    case "failed":
      return t("desk.job.failed");
    case "cancelled":
      return t("desk.job.cancelled");
    case "closed":
      return t("desk.job.closed");
    case "moved_out":
      return t("desk.job.moved_out");
    case "archived":
      return t("desk.job.archived");
  }
}

const REASON: Record<string, MessageKey> = {
  spawn_failed: "desk.jobReason.spawn_failed",
  exited: "desk.jobReason.exited",
  device_disconnected: "desk.jobReason.device_disconnected",
};

/** A failed job's reason, from its code; a code this build does not know reads as the general failure. */
export function jobReasonWords(t: T, reason: string | null | undefined): string | null {
  if (!reason) return null;
  const key = REASON[reason];
  return key ? t(key) : null;
}

/** The second line's words after the place (B24): the verb and what follows it. */
function jobWords(t: T, job: DeskJob): { verb: string | null; words: string | null; tone: "warning" | "destructive" | null; mono: boolean } {
  if (job.disconnected) return { verb: t("desk.job.disconnected"), words: job.now ?? null, tone: null, mono: false };
  switch (job.state) {
    case "working":
      return { verb: null, words: job.now ?? t("desk.job.working"), tone: null, mono: false };
    case "asking":
      return { verb: jobStateWords(t, job), words: job.question ?? null, tone: "warning", mono: job.ask === "approval" };
    case "done":
      return { verb: t("desk.job.done"), words: job.summary ?? null, tone: null, mono: false };
    case "failed":
      return { verb: t("desk.job.failed"), words: jobReasonWords(t, job.reason), tone: "destructive", mono: false };
    default:
      return { verb: jobStateWords(t, job), words: null, tone: null, mono: false };
  }
}

/** The second line as one string, for a hint or a list that takes text. */
export function jobLine(t: T, job: DeskJob): string {
  const { verb, words } = jobWords(t, job);
  return [verb, words].filter(Boolean).join(" · ");
}

/**
 * A job's second line (B24, B28, D-75), the sidebar row's words: where it
 * runs (a worktree's branch, else the project), then its verb and words, a
 * wait for the operator in the warning colour and a failure in the
 * destructive one. `place` is left out under a Conductor, whose workers share
 * its project.
 */
export function JobLine({ job, place = true, className }: { job: DeskJob; place?: boolean; className?: string }) {
  const { t } = useInterfaceTranslation();
  const { verb, words, tone, mono } = jobWords(t, job);
  const toned = tone === "warning" ? "text-warning" : tone === "destructive" ? "text-destructive" : null;
  return (
    <span className={cn("flex min-w-0 items-center gap-xxs text-caption text-muted-foreground", className)} title={jobLine(t, job)} data-desk-job-line="true">
      {place ? (
        <span className="flex min-w-0 max-w-1/2 shrink-0 items-center gap-xxs">
          {job.location.kind === "worktree" ? <GitBranchIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" /> : null}
          <span className="truncate">{job.location.kind === "worktree" ? job.location.label : job.project.label}</span>
          <span aria-hidden="true">·</span>
        </span>
      ) : null}
      <span className={cn("min-w-0 truncate", toned)}>
        {verb}
        {verb && words ? " · " : null}
        {words ? <span className={cn(mono && "font-mono")}>{words}</span> : null}
      </span>
    </span>
  );
}

/** The mark at a job's left (D-75): its sidebar row's status mark, hollow for an ask already read; a job with no row leaves the slot empty. */
export function JobMark({ job }: { job: DeskJob }) {
  return <StatusMark mark={job.mark} read={job.mark_read} data-desk-job-mark={job.state} />;
}

/** Whether a job has a session behind it to show the agent of: not one still queued, and not one that failed before it started. */
function hasSession(job: DeskJob): boolean {
  return job.state !== "queued" && !(job.state === "failed" && job.reason === "spawn_failed");
}

/** How a session starts wherever it is drawn (D-75): its status and its agent. */
export function JobLead({ job }: { job: DeskJob }) {
  return (
    <>
      <JobMark job={job} />
      {hasSession(job) ? <AgentMark kind={job.agent_kind} /> : <span aria-hidden="true" className="size-(--size-agent-mark) shrink-0" />}
    </>
  );
}

/** A session named inside a line (a letter's other side, a task handed on): its status, its agent and its name, on the chip ground (B26, D-75); a session whose state Hide does not know shows its name alone. */
export function SessionChip({ session, onPress, className }: { session: DeskSessionRef; onPress?: () => void; className?: string }) {
  const { t } = useInterfaceTranslation();
  const lead = session.agent_kind === null ? null : <><StatusMark mark={session.mark} /><AgentMark kind={session.agent_kind} /></>;
  const look = cn("inline-flex max-w-full min-w-0 items-center gap-xxs rounded-xs bg-secondary px-xxs align-middle text-foreground", className);
  if (!onPress) return <span className={look} data-desk-session-chip={session.name}>{lead}<span className="truncate">{session.name}</span></span>;
  return (
    <Button variant="text" size="inline" className={cn(look, "hover:bg-accent hover:text-foreground")} aria-label={t("desk.mention.open", { name: session.name })} data-desk-session-chip={session.name} onClick={onPress}>
      {lead}
      <span className="truncate">{session.name}</span>
    </Button>
  );
}

/** The dot on an output not looked at yet (the Views chip's unseen dot, D-64), named for a screen reader. */
export function UnseenDot({ className }: { className?: string }) {
  const { t } = useInterfaceTranslation();
  return <span role="img" aria-label={t("desk.outputs.unseen")} className={cn("size-(--size-tab-status-dot) shrink-0 rounded-full bg-foreground", className)} data-desk-output-unseen="true" />;
}

/** The mark of an output's kind: an image, a page, a PR, or the document's file type. */
export function OutputMark({ output, className }: { output: DeskOutput; className?: string }) {
  const size = cn("size-(--size-icon-sm) shrink-0 text-muted-foreground", className);
  if (output.kind === "image") return <ImageIcon aria-hidden="true" className={size} />;
  if (output.kind === "page") return <GlobeIcon aria-hidden="true" className={size} />;
  if (output.kind === "pr") {
    if (!output.pr_state) throw new Error(`PR output ${output.id} carries no pr_state`);
    return <PrMark state={output.pr_state} compact className="shrink-0" />;
  }
  return <FileMark icon={fileIcon(output.label)} className={size} />;
}

/**
 * A job's outputs as one chip (B31, D-75): the selected or first one, `+N`
 * for the rest, and a dot while any is unseen. Pressing it opens them in the
 * lane's 보기 column.
 */
export function OutputsChip({ outputs, selected = null, onOpen }: { outputs: readonly DeskOutput[]; selected?: string | null; onOpen: (output: DeskOutput) => void }) {
  const shown = outputs.find((output) => output.id === selected) ?? outputs[0];
  if (!shown) return null;
  const more = outputs.length - 1;
  return (
    <Button
      variant="text"
      size="inline"
      className={cn("relative h-(--size-badge-height) max-w-full gap-xxs rounded-xs bg-secondary px-xs text-micro text-subtle-foreground hover:bg-accent hover:text-foreground", selected === shown.id && "ring-1 ring-ring")}
      aria-label={outputs.map((output) => output.label).join(", ")}
      data-desk-output-chip={shown.id}
      onClick={() => onOpen(shown)}
    >
      <OutputMark output={shown} />
      <span className="truncate">{shown.label}</span>
      {more > 0 ? <span className="shrink-0 text-muted-foreground" data-desk-job-more-outputs={more}>+{more}</span> : null}
      {outputs.some((output) => !output.seen) ? <UnseenDot /> : null}
    </Button>
  );
}

const MISSION_WORDS: Record<DeskMission["status"], MessageKey> = {
  planning: "mission.chip.plan",
  running: "mission.chip",
  merge_wait: "mission.chip.mergeWait",
  done: "mission.chip.done",
  cancelled: "mission.chip.cancelled",
};

/** A Conductor's `미션 k/n` chip (B34), drawn from the mission the job carries. */
export function DeskMissionChip({ mission, onOpen }: { mission: DeskMission; onOpen: (missionId: string) => void }) {
  const { t } = useInterfaceTranslation();
  return <MissionChipView id={mission.id} title={mission.title} words={t(MISSION_WORDS[mission.status])} counts={mission.status === "running" ? `${mission.done}/${mission.total}` : null} tone={mission.tone} deciding={mission.deciding} status={mission.status} onOpen={onOpen} />;
}

/** A job's plan as the pane head's plan chip draws it (B24): its cells, `n/m`, and `검토` while it waits for review. */
export function DeskPlanChip({ plan, review }: { plan: NonNullable<DeskJob["plan"]>; review: boolean }) {
  const finished = !review && plan.done >= plan.total;
  const tones: CellTone[] = Array.from({ length: plan.total }, (_, index) => (index < plan.done ? "done" : index === plan.done ? (review ? "hold" : "working") : "waiting"));
  return (
    <span className={planChipLook(review, finished)} data-desk-plan-chip={review ? "review" : finished ? "finished" : "running"}>
      <PlanChipFace tones={tones} count={`${plan.done}/${plan.total}`} proposed={review} finished={finished} />
    </span>
  );
}

export type DeskJobCardActions = {
  onOpen: (job: DeskJob) => void;
  onAnswer: (job: DeskJob) => void;
  onCancel: (job: DeskJob) => void;
  onRequeue: (job: DeskJob) => void;
  onRetry: (job: DeskJob) => void;
  onOutput: (output: DeskOutput) => void;
  onMission: (missionId: string) => void;
};

/**
 * The one chip at a card's or a row's right end (D-75): what matters most in
 * its state, in this order: its mission, its plan, the device it runs on,
 * the operator joining it, its outputs, its PR.
 */
export function JobChip({ job, selectedOutput = null, onOutput, onMission }: { job: DeskJob; selectedOutput?: string | null; onOutput: (output: DeskOutput) => void; onMission: (missionId: string) => void }) {
  const { t } = useInterfaceTranslation();
  if (job.mission) return <DeskMissionChip mission={job.mission} onOpen={onMission} />;
  if (job.plan && (job.state === "working" || job.ask === "plan_review")) return <DeskPlanChip plan={job.plan} review={job.ask === "plan_review"} />;
  if (job.machine) return <DeviceChip label={job.machine} />;
  if (job.joined) return <Badge variant="outline" data-desk-job-joined="true">{t("desk.job.joined")}</Badge>;
  if (job.outputs.length) return <OutputsChip outputs={job.outputs} selected={selectedOutput} onOpen={onOutput} />;
  const pr = job.pull_requests[0];
  return pr ? <PrMark state={pr.state} number={pr.number} /> : null;
}

/** The words at the second line's end (D-75): the one thing the operator would do, as words; a card with nothing to do has none. */
function action(t: T, job: DeskJob, actions: DeskJobCardActions): ReactNode {
  const words = (kind: string, label: string, onPress: () => void, more = true) => (
    <Button variant="text" size="inline" className="pointer-events-auto relative shrink-0 gap-0 text-caption text-foreground" data-desk-job-action={kind} onClick={onPress}>
      {label}
      {more ? <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" /> : null}
    </Button>
  );
  switch (job.state) {
    case "asking":
      return words("answer", t("desk.job.answer"), () => actions.onAnswer(job));
    case "failed":
      return words("retry", t("desk.action.retry"), () => actions.onRetry(job));
    case "queued":
      return words("cancel", t("desk.job.cancel"), () => actions.onCancel(job), false);
    case "cancelled":
      return words("requeue", t("desk.job.requeue"), () => actions.onRequeue(job));
    default:
      return null;
  }
}

/**
 * One job Desk started, as a card in the chat (D-39, D-75, B21-B24,
 * B36-B39): an unfilled card of two lines. The first is the session (its
 * status, its agent, its title) and the one chip that matters most; the
 * second is where it runs and what it does, asks or left, and the one thing
 * to do, as words at its end. Pressing the card opens its thread (B25); the
 * selected card sits on the selection ground. An ended job and a job on a
 * device that dropped are dimmed.
 */
export function DeskJobCard({ job, selected = false, selectedOutput = null, ...actions }: { job: DeskJob; selected?: boolean; selectedOutput?: string | null } & DeskJobCardActions) {
  const { t } = useInterfaceTranslation();
  const ended = job.state === "cancelled" || job.state === "closed" || job.state === "moved_out" || job.state === "archived";
  return (
    <article
      className={cn("relative min-w-0 rounded-lg border border-border", selected ? "bg-accent" : "hover:bg-accent/50")}
      aria-label={job.title}
      data-desk-job={job.state}
      data-desk-job-selected={selected ? "true" : undefined}
    >
      {job.state !== "moved_out" ? <RowOverlayButton className="rounded-lg" hover={false} aria-label={t("desk.job.open", { title: job.title })} onClick={() => actions.onOpen(job)} /> : null}
      <div className={cn("pointer-events-none relative flex min-w-0 flex-col gap-xxs px-md py-sm", (ended || job.disconnected) && "opacity-(--opacity-dimmed)")}>
        <div className="flex min-h-(--size-sidebar-line) min-w-0 items-center gap-xs">
          <JobLead job={job} />
          <span className="min-w-0 flex-1 truncate font-medium text-foreground">{job.title}</span>
          <span className="pointer-events-auto relative flex shrink-0 items-center gap-xxs">
            <JobChip job={job} selectedOutput={selectedOutput} onOutput={actions.onOutput} onMission={actions.onMission} />
          </span>
        </div>
        <div className="flex min-w-0 items-center gap-xs">
          <span aria-hidden="true" className="w-(--size-agent-mark) shrink-0" />
          <JobLine job={job} className="flex-1" />
          {action(t, job, actions)}
        </div>
      </div>
    </article>
  );
}
