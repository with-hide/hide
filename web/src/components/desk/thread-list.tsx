import { ChevronDownIcon, ChevronRightIcon } from "lucide-react";
import type { DeskJob, ThreadGroupKey } from "../../desk";
import { threadGroups } from "../../desk";
import { AgentMark } from "../../AgentMark";
import { useInterfaceTranslation } from "../../i18n/client";
import type { MessageKey } from "../../i18n/catalogs";
import { cn } from "../../lib/utils";
import { Elapsed } from "../elapsed";
import { PrMark } from "../pr-mark";
import { SessionRow } from "../session-row";
import { Button } from "../ui/button";
import { DeskMissionChip, JobMark, jobLine } from "./desk-job-card";

const GROUP: Record<ThreadGroupKey, MessageKey> = {
  turn: "desk.threads.group.turn",
  working: "desk.threads.group.working",
  done: "desk.threads.group.done",
};

/** One thread as the sidebar's task row (D-71, B28): its marks, title, mission and PRs, time, and the project with its state words under it. */
export function ThreadRow({ job, selected, expanded, worker = false, onOpen, onMission, onToggle }: {
  job: DeskJob;
  selected: boolean;
  expanded?: boolean;
  worker?: boolean;
  onOpen: (job: DeskJob) => void;
  onMission: (missionId: string) => void;
  onToggle?: () => void;
}) {
  const { t } = useInterfaceTranslation();
  const ended = job.state !== "asking" && job.state !== "working" && job.state !== "queued" && job.state !== "starting" && !(job.state === "done" && !job.read);
  return (
    <SessionRow
      mark={<>
        {onToggle ? (
          <Button variant="row-action" size="icon-xs" className="-ml-xxs" aria-expanded={expanded} aria-label={t(expanded ? "desk.threads.hideWorkers" : "desk.threads.showWorkers")} data-desk-thread-toggle={expanded ? "open" : "closed"} onClick={onToggle}>
            {expanded ? <ChevronDownIcon /> : <ChevronRightIcon />}
          </Button>
        ) : null}
        <JobMark job={job} />
        <AgentMark kind={job.agent_kind ?? null} />
      </>}
      title={job.title}
      titleClassName={ended ? "text-subtle-foreground" : "text-foreground"}
      chips={<>
        {job.mission ? <DeskMissionChip mission={job.mission} onOpen={onMission} /> : null}
        {job.pull_requests.map((pr) => <PrMark key={pr.number} state={pr.state} number={pr.number} />)}
      </>}
      right={<Elapsed since={job.changed_at_unix_ms} className="font-mono text-caption text-muted-foreground" />}
      line={{
        text: worker ? jobLine(t, job) : `${job.location.kind === "worktree" ? job.location.label : job.project.label} · ${jobLine(t, job)}`,
        className: job.state === "asking" ? "text-warning" : job.state === "failed" ? "text-destructive" : undefined,
        wrap: job.state === "asking",
      }}
      onPress={() => onOpen(job)}
      pressLabel={t("desk.job.open", { title: job.title })}
      className={cn(selected && "bg-accent", worker && "pl-(--size-tree-indent)", job.disconnected && "opacity-(--opacity-dimmed)")}
      aria-current={selected ? "true" : undefined}
      data-desk-thread={job.id}
      data-desk-thread-state={job.state}
    />
  );
}

/**
 * The Threads list (D-64, D-71, B28): Desk's threads as the sidebar's task
 * rows, in 내 차례, 작업 중 and 끝남; a Conductor row opens to its workers,
 * indented under it. A thread pulled into the operator's tabs is gone from
 * it (B37). `expanded` holds the Conductors shown open.
 */
export function ThreadList({ jobs, selected = null, expanded, onOpen, onMission, onToggle }: {
  jobs: readonly DeskJob[];
  selected?: string | null;
  expanded: ReadonlySet<string>;
  onOpen: (job: DeskJob) => void;
  onMission: (missionId: string) => void;
  onToggle: (jobId: string) => void;
}) {
  const { t } = useInterfaceTranslation();
  const groups = threadGroups(jobs);
  const byId = new Map(jobs.map((job) => [job.id, job]));
  if (groups.length === 0) return <p className="px-md py-lg text-caption text-muted-foreground" data-desk-threads-empty="true">{t("desk.threads.empty")}</p>;
  return (
    <div className="flex min-w-0 flex-col gap-sm px-xs py-xs" data-desk-threads={jobs.length}>
      {groups.map((group) => (
        <section key={group.key} className="flex min-w-0 flex-col" aria-label={t(GROUP[group.key], { count: group.jobs.length })} data-desk-thread-group={group.key}>
          <h3 className={cn("px-xs pb-xxs text-caption font-medium", group.key === "turn" ? "text-warning" : "text-muted-foreground")}>{t(GROUP[group.key], { count: group.jobs.length })}</h3>
          <ol className="flex min-w-0 flex-col">
            {group.jobs.map((job) => {
              const workers = (job.workers ?? []).map((id) => byId.get(id)).filter((one): one is DeskJob => one !== undefined);
              const open = expanded.has(job.id);
              return [
                <ThreadRow key={job.id} job={job} selected={selected === job.id} expanded={open} onOpen={onOpen} onMission={onMission} onToggle={workers.length ? () => onToggle(job.id) : undefined} />,
                ...(open ? workers.map((worker) => <ThreadRow key={worker.id} job={worker} selected={selected === worker.id} worker onOpen={onOpen} onMission={onMission} />) : []),
              ];
            })}
          </ol>
        </section>
      ))}
    </div>
  );
}
