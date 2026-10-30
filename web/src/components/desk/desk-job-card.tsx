import { CheckIcon, CircleXIcon, GlobeIcon, HandIcon, ImageIcon } from "lucide-react";
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

/** The dot on an output not looked at yet, named for a screen reader. */
export function UnseenDot({ className }: { className?: string }) {
  const { t } = useInterfaceTranslation();
  return <span role="img" aria-label={t("desk.outputs.unseen")} className={cn("size-(--size-tab-status-dot) shrink-0 rounded-full bg-primary", className)} data-desk-output-unseen="true" />;
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

/** An output as a chip on a card (B31): pressing it opens it in the lane's 보기 column, as from the thread. */
export function OutputChip({ output, selected = false, onOpen }: { output: DeskOutput; selected?: boolean; onOpen: (output: DeskOutput) => void }) {
  return (
    <Button
      variant="outline"
      size="sm"
      className={cn("pointer-events-auto relative h-(--size-badge-height) max-w-full gap-xxs bg-transparent px-xs text-micro font-normal text-subtle-foreground", selected && "border-ring")}
      aria-pressed={selected}
      data-desk-output-chip={output.id}
      onClick={() => onOpen(output)}
    >
      <OutputMark output={output} />
      <span className="truncate">{output.label}</span>
      {!output.seen ? <UnseenDot /> : null}
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
          <span role="img" className={cn("size-(--size-status-mark) rounded-xs", TONE_FILL[node.tone], node.tone === "waiting" && "border border-border")} aria-label={node.title} data-desk-mission-node={node.tone} />
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

/** What a card offers for its state (B21, B22, B29): the one action the operator would take, as a grey button. */
function action(t: T, job: DeskJob, actions: DeskJobCardActions): ReactNode {
  const button = (kind: string, words: string, onPress: () => void) => (
    <Button variant="soft" size="sm" className="pointer-events-auto relative font-normal" data-desk-job-action={kind} onClick={onPress}>{words}</Button>
  );
  switch (job.state) {
    case "asking":
      return button("answer", t("desk.job.answer"), () => actions.onAnswer(job));
    case "done":
      return button("open", t("desk.job.viewThread"), () => actions.onOpen(job));
    case "queued":
      return button("cancel", t("desk.job.cancel"), () => actions.onCancel(job));
    case "cancelled":
      return button("requeue", t("desk.job.requeue"), () => actions.onRequeue(job));
    case "failed":
      return button("retry", t("desk.action.retry"), () => actions.onRetry(job));
    default:
      return null;
  }
}

/** A card's one coloured mark (D-39): ✋ while it asks, a check when done, a cross when it failed, else its status. */
function CardMark({ job }: { job: DeskJob }) {
  if (job.state === "asking") return <HandIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-warning" data-desk-job-mark="asking" />;
  if (job.state === "done") return <CheckIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-success" data-desk-job-mark="done" />;
  return <JobMark job={job} />;
}

/** The card's second line (B24): the question, what it does now, its summary or its state, then where it runs. */
function cardLine(t: T, job: DeskJob): string {
  const what = job.state === "asking" ? (job.question ?? t("desk.job.asking"))
    : job.state === "working" ? (job.now ?? t("desk.job.working"))
    : job.state === "failed" ? jobLine(t, job)
    : job.state === "done" ? (job.summary ?? t("desk.job.done"))
    : jobStateWords(t, job);
  const place = job.location.kind === "worktree" ? `${job.project.label} · ${job.location.label}` : job.project.label;
  return [what, place, job.machine].filter(Boolean).join(" · ");
}

/**
 * One job Desk started, as a card in the chat (D-39, B21-B24, B36-B39): an
 * unfilled card with a thin line, the one coloured mark of its state, its
 * title with its mission and PRs, an output at the top right, one quiet
 * line of what it asks, does or left and where, and the one grey action its
 * state offers. Pressing the card opens its thread (B25). An ended job is
 * dimmed; a job on a device that dropped is dimmed and says so (B23).
 */
export function DeskJobCard({ job, selected = false, selectedOutput = null, ...actions }: { job: DeskJob; selected?: boolean; selectedOutput?: string | null } & DeskJobCardActions) {
  const { t } = useInterfaceTranslation();
  const ended = job.state === "cancelled" || job.state === "closed" || job.state === "moved_out" || job.state === "archived";
  const button = action(t, job, actions);
  const plan = job.state === "working" && job.plan ? `${job.plan.done}/${job.plan.total}` : null;
  // One output stands at the top right, the selected one if any; the rest are counted and listed in the thread's 결과물.
  const shown = job.outputs.find((output) => output.id === selectedOutput) ?? job.outputs[0] ?? null;
  const more = job.outputs.filter((output) => output !== shown);
  return (
    <article
      className={cn("relative min-w-0 rounded-lg border border-border", selected && "border-ring")}
      aria-label={job.title}
      data-desk-job={job.state}
      data-desk-job-selected={selected ? "true" : undefined}
    >
      {job.state !== "moved_out" ? <RowOverlayButton className="rounded-lg" aria-label={t("desk.job.open", { title: job.title })} onClick={() => actions.onOpen(job)} /> : null}
      <div className={cn("pointer-events-none relative grid min-w-0 grid-cols-[auto_minmax(0,1fr)_auto] items-start gap-x-sm gap-y-xxs px-md py-sm", (ended || job.disconnected) && "opacity-(--opacity-dimmed)")}>
        <span className="flex h-(--size-control-sm) items-center"><CardMark job={job} /></span>
        <span className="flex h-(--size-control-sm) min-w-0 items-center gap-xs">
          <span className={cn("min-w-0 truncate font-medium", ended ? "text-subtle-foreground" : "text-foreground")}>{job.title}</span>
          {job.mission ? <span className="pointer-events-auto relative"><DeskMissionChip mission={job.mission} onOpen={actions.onMission} /></span> : null}
          {job.pull_requests.map((pr) => <PrMark key={pr.number} state={pr.state} number={pr.number} />)}
          {job.joined ? <Badge variant="outline" data-desk-job-joined="true">{t("desk.job.joined")}</Badge> : null}
        </span>
        <span className="flex h-(--size-control-sm) min-w-0 items-center justify-end gap-xxs">
          {plan ? <span className="inline-flex h-(--size-badge-height) shrink-0 items-center rounded-sm border border-border px-xs font-mono text-micro text-muted-foreground" data-desk-job-plan={plan}>{plan}</span> : null}
          {shown ? <OutputChip output={shown} selected={selectedOutput === shown.id} onOpen={actions.onOutput} /> : null}
          {more.length ? <span className="shrink-0 text-micro text-muted-foreground" title={more.map((output) => output.label).join(", ")} data-desk-job-more-outputs={more.length}>+{more.length}</span> : null}
        </span>
        <p className="col-span-2 col-start-2 min-w-0 truncate text-caption text-muted-foreground" title={cardLine(t, job)} data-desk-job-line="true">
          {job.disconnected ? <span className="text-warning" data-desk-job-disconnected="true">{t("desk.job.disconnected")} · </span> : null}
          {cardLine(t, job)}
        </p>
        {job.mission ? <span className="col-span-2 col-start-2 pt-xxs"><MissionNodeCells mission={job.mission} /></span> : null}
        {button ? <span className="col-span-2 col-start-2 pt-xs">{button}</span> : null}
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
    <article className="relative min-w-0 rounded-lg border border-border" aria-label={title} data-desk-handed="true">
      <RowOverlayButton className="rounded-lg" aria-label={t("desk.job.open", { title: name })} onClick={onOpen} />
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
