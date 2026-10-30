import { CircleXIcon, FolderIcon, GitBranchIcon, GlobeIcon, ImageIcon, ServerIcon } from "lucide-react";
import type { ReactNode } from "react";
import type { DeskJob, DeskMission, DeskOutput } from "../../desk";
import { jobMark } from "../../desk";
import { fileIcon } from "../../fileIcons";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import type { MessageKey } from "../../i18n/catalogs";
import { FileMark } from "../file-mark";
import { MissionChipView } from "../mission-chip";
import { TONE_FILL } from "../mission-tone";
import { PrMark } from "../pr-mark";
import { StatusMark } from "../status-mark";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { RowOverlayButton } from "../ui/row-button";
import { Hint } from "../ui/tooltip";

type T = ReturnType<typeof useInterfaceTranslation>["t"];

/** A job's state in its words (B21-B24, B36-B39): what the card's second line and the Threads row say first. */
export function jobStateWords(t: T, job: DeskJob): string {
  switch (job.state) {
    case "queued":
      return t("desk.job.queued", { position: job.queue_position ?? 0 });
    case "starting":
      return t("desk.job.starting");
    case "working":
      return t("desk.job.working");
    case "asking":
      return t("desk.job.asking");
    case "done":
      return t("desk.job.done");
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

/**
 * The line under a job's title (B24): what it is doing now and its plan while
 * it works, its question while it asks, its last turn's summary when done, a
 * failure's reason, otherwise its state.
 */
export function jobLine(t: T, job: DeskJob): string {
  const plan = job.plan ? t("desk.job.plan", { done: job.plan.done, total: job.plan.total }) : null;
  switch (job.state) {
    case "working":
      return [job.now ?? t("desk.job.working"), plan].filter(Boolean).join(" · ");
    case "asking":
      return [t("desk.job.asking"), job.question].filter(Boolean).join(" · ");
    case "done":
      return job.summary ?? t("desk.job.done");
    case "failed":
      return [t("desk.job.failed"), jobReasonWords(t, job.reason)].filter(Boolean).join(" · ");
    default:
      return jobStateWords(t, job);
  }
}

/** The mark at a job's left: its status, or a failure's cross, which no status mark draws. */
export function JobMark({ job }: { job: DeskJob }) {
  if (job.state === "failed") return <CircleXIcon aria-hidden="true" className="size-(--size-agent-mark) shrink-0 text-destructive" data-desk-job-mark="failed" />;
  return <StatusMark mark={jobMark(job)} data-desk-job-mark={job.state} />;
}

/** Where a job runs (D-19, B17): its checkout or the worktree's branch, as the lane head names a scope. */
export function LocationChip({ location }: { location: DeskJob["location"] }) {
  const { t } = useInterfaceTranslation();
  const Icon = location.kind === "worktree" ? GitBranchIcon : FolderIcon;
  const label = location.kind === "worktree" ? t("desk.job.worktree", { label: location.label }) : t("desk.job.checkout", { label: location.label });
  return (
    <Hint label={label}>
      <span className="inline-flex min-w-0 shrink items-center gap-xxs text-micro text-subtle-foreground" aria-label={label} data-desk-location={location.kind}>
        <Icon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
        <span className="truncate">{location.label}</span>
      </span>
    </Hint>
  );
}

/** The mark of an output's kind: an image, a page, a PR, or the document's file type. */
export function OutputMark({ output, className }: { output: DeskOutput; className?: string }) {
  const size = cn("size-(--size-icon-sm) shrink-0 text-muted-foreground", className);
  if (output.kind === "image") return <ImageIcon aria-hidden="true" className={size} />;
  if (output.kind === "page") return <GlobeIcon aria-hidden="true" className={size} />;
  if (output.kind === "pr") return <PrMark state="pending" compact className="shrink-0" />;
  return <FileMark icon={fileIcon(output.label)} className={size} />;
}

/** An output as a chip on a card (B31): pressing it opens it in the lane's 보기 column, as from the thread. */
export function OutputChip({ output, selected = false, onOpen }: { output: DeskOutput; selected?: boolean; onOpen: (output: DeskOutput) => void }) {
  return (
    <Button
      variant="secondary"
      size="sm"
      className={cn("pointer-events-auto relative h-(--size-badge-height) max-w-full gap-xxs px-xs text-micro font-normal", selected && "border-ring")}
      aria-pressed={selected}
      data-desk-output-chip={output.id}
      onClick={() => onOpen(output)}
    >
      <OutputMark output={output} />
      <span className="truncate">{output.label}</span>
      {!output.seen ? <span aria-hidden="true" className="size-(--size-tab-status-dot) shrink-0 rounded-full bg-primary" data-desk-output-unseen="true" /> : null}
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

/** A mission's nodes as a row of cells in their state colours, each named in its hint (B34). */
export function MissionNodeCells({ mission }: { mission: DeskMission }) {
  return (
    <span className="flex items-center gap-xxs" data-desk-mission-nodes={mission.nodes.length}>
      {mission.nodes.map((node) => (
        <Hint key={node.key} label={node.title}>
          <span className={cn("size-(--size-status-mark) rounded-xs", TONE_FILL[node.tone], node.tone === "waiting" && "border border-border")} aria-label={node.title} data-desk-mission-node={node.tone} />
        </Hint>
      ))}
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

/** What a card offers for its state (B21, B22, B29): the one action the operator would take. */
function action(t: T, job: DeskJob, actions: DeskJobCardActions): ReactNode {
  switch (job.state) {
    case "asking":
      return <Button size="sm" className="pointer-events-auto relative" data-desk-job-action="answer" onClick={() => actions.onAnswer(job)}>{t("desk.job.answer")}</Button>;
    case "queued":
      return <Button variant="outline" size="sm" className="pointer-events-auto relative" data-desk-job-action="cancel" onClick={() => actions.onCancel(job)}>{t("desk.job.cancel")}</Button>;
    case "cancelled":
      return <Button variant="outline" size="sm" className="pointer-events-auto relative" data-desk-job-action="requeue" onClick={() => actions.onRequeue(job)}>{t("desk.job.requeue")}</Button>;
    case "failed":
      return <Button variant="outline" size="sm" className="pointer-events-auto relative" data-desk-job-action="retry" onClick={() => actions.onRetry(job)}>{t("desk.action.retry")}</Button>;
    default:
      return null;
  }
}

/**
 * One job Desk started, as a card in the chat (D-39, B21-B24, B36-B39): its
 * mark and title, where it runs, its mission and PRs, the line of what it is
 * doing, asking or left, its outputs and the one action its state offers.
 * Pressing the card opens its thread (B25). An ended job is dimmed; a job
 * on a device that dropped is dimmed and says so (B23).
 */
export function DeskJobCard({ job, selected = false, selectedOutput = null, ...actions }: { job: DeskJob; selected?: boolean; selectedOutput?: string | null } & DeskJobCardActions) {
  const { t } = useInterfaceTranslation();
  const ended = job.state === "cancelled" || job.state === "closed" || job.state === "moved_out" || job.state === "archived";
  const line = jobLine(t, job);
  const button = action(t, job, actions);
  return (
    <article
      className={cn("relative min-w-0 rounded-md border border-border bg-card", selected && "border-ring")}
      aria-label={job.title}
      data-desk-job={job.state}
      data-desk-job-selected={selected ? "true" : undefined}
    >
      {job.state !== "moved_out" ? <RowOverlayButton className="rounded-md" aria-label={t("desk.job.open", { title: job.title })} onClick={() => actions.onOpen(job)} /> : null}
      <div className={cn("pointer-events-none relative flex min-w-0 flex-col gap-xxs px-md py-sm", (ended || job.disconnected) && "opacity-(--opacity-dimmed)")}>
        <div className="flex min-w-0 items-center gap-xs">
          <JobMark job={job} />
          <span className={cn("min-w-0 truncate font-medium", ended ? "text-subtle-foreground" : "text-foreground")}>{job.title}</span>
          {job.mission ? <span className="pointer-events-auto relative"><DeskMissionChip mission={job.mission} onOpen={actions.onMission} /></span> : null}
          {job.pull_requests.map((pr) => <PrMark key={pr.number} state={pr.state} number={pr.number} />)}
          {job.joined ? <Badge variant="outline" data-desk-job-joined="true">{t("desk.job.joined")}</Badge> : null}
        </div>
        <div className="ml-xs flex min-w-0 items-center gap-sm pl-(--size-agent-mark) text-micro">
          {/* ⌂ is always drawn (B17); the project is named beside it only where the place does not already say it. */}
          {job.location.kind === "worktree" || job.location.label !== job.project.label ? <span className="min-w-0 truncate text-subtle-foreground">{job.project.label}</span> : null}
          <LocationChip location={job.location} />
          {job.machine ? <span className="inline-flex shrink-0 items-center gap-xxs text-subtle-foreground" data-desk-job-machine={job.machine}><ServerIcon aria-hidden="true" className="size-(--size-icon-sm)" />{job.machine}</span> : null}
          {job.disconnected ? <span className="shrink-0 text-warning" data-desk-job-disconnected="true">{t("desk.job.disconnected")}</span> : null}
        </div>
        <p
          className={cn("ml-xs min-w-0 whitespace-normal break-keep pl-(--size-agent-mark) text-caption [overflow-wrap:anywhere]", job.state === "asking" ? "text-warning" : job.state === "failed" ? "text-destructive" : "text-muted-foreground")}
          data-desk-job-line="true"
        >
          {line}
        </p>
        {job.mission ? <span className="ml-xs pl-(--size-agent-mark) pt-xxs"><MissionNodeCells mission={job.mission} /></span> : null}
        {job.outputs.length > 0 || button ? (
          <div className="ml-xs flex min-w-0 flex-wrap items-center gap-xs pl-(--size-agent-mark) pt-xs">
            {job.outputs.map((output) => <OutputChip key={output.id} output={output} selected={selectedOutput === output.id} onOpen={actions.onOutput} />)}
            {button ? <span className="ml-auto">{button}</span> : null}
          </div>
        ) : null}
      </div>
    </article>
  );
}

/**
 * A task Desk handed to a session that was already doing it (D-24, B16): a
 * notice, not a job, so it never changes state and holds no cap place;
 * pressing it opens that session's thread.
 */
export function DeskHandedCard({ name, project, onOpen }: { name: string; project: string; onOpen: () => void }) {
  const { t } = useInterfaceTranslation();
  const title = t("desk.handed.title", { name });
  return (
    <article className="relative min-w-0 rounded-md border border-border bg-card" aria-label={title} data-desk-handed="true">
      <RowOverlayButton className="rounded-md" aria-label={t("desk.job.open", { title: name })} onClick={onOpen} />
      <div className="pointer-events-none relative flex min-w-0 flex-col gap-xxs px-md py-sm">
        <div className="flex min-w-0 items-center gap-xs">
          <StatusMark mark={null} />
          <span className="min-w-0 truncate font-medium text-subtle-foreground">{title}</span>
        </div>
        <span className="ml-xs min-w-0 truncate pl-(--size-agent-mark) text-micro text-muted-foreground">{project}</span>
      </div>
    </article>
  );
}
