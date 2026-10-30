import { ChevronDownIcon, ChevronRightIcon } from "lucide-react";
import type { DeskJob, DeskOutput, ThreadGroupKey } from "../../desk";
import { threadGroups } from "../../desk";
import { useInterfaceTranslation } from "../../i18n/client";
import type { MessageKey } from "../../i18n/catalogs";
import { cn } from "../../lib/utils";
import { Elapsed } from "../elapsed";
import { SessionRow } from "../session-row";
import { Button } from "../ui/button";
import { JobChip, JobLead, JobLine, jobLine } from "./desk-job-card";

const GROUP: Record<ThreadGroupKey, MessageKey> = {
  turn: "desk.threads.group.turn",
  working: "desk.threads.group.working",
  done: "desk.threads.group.done",
};

/**
 * One thread as the sidebar's task row (D-71, D-75, B28): its status and
 * agent, its title, the one chip a card shows and its time; under it where it
 * runs and what it does, as a card's second line. A worker under its
 * Conductor leaves out the place they share.
 */
export function ThreadRow({ job, selected, expanded, worker = false, onOpen, onMission, onOutput, onToggle }: {
  job: DeskJob;
  selected: boolean;
  expanded?: boolean;
  worker?: boolean;
  onOpen: (job: DeskJob) => void;
  onMission: (missionId: string) => void;
  onOutput: (output: DeskOutput) => void;
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
        <JobLead job={job} />
      </>}
      title={job.title}
      titleClassName={ended ? "text-subtle-foreground" : "text-foreground"}
      chips={<span className="relative flex shrink-0 items-center"><JobChip job={job} onOutput={onOutput} onMission={onMission} /></span>}
      right={<Elapsed since={job.changed_at_unix_ms} className="font-mono text-caption text-muted-foreground" />}
      hint={jobLine(t, job)}
      below={<JobLine job={job} place={!worker} />}
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
export function ThreadList({ jobs, selected = null, expanded, onOpen, onMission, onOutput, onToggle }: {
  jobs: readonly DeskJob[];
  selected?: string | null;
  expanded: ReadonlySet<string>;
  onOpen: (job: DeskJob) => void;
  onMission: (missionId: string) => void;
  onOutput: (output: DeskOutput) => void;
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
                <ThreadRow key={job.id} job={job} selected={selected === job.id} expanded={open} onOpen={onOpen} onMission={onMission} onOutput={onOutput} onToggle={workers.length ? () => onToggle(job.id) : undefined} />,
                ...(open ? workers.map((worker) => <ThreadRow key={worker.id} job={worker} selected={selected === worker.id} worker onOpen={onOpen} onMission={onMission} onOutput={onOutput} />) : []),
              ];
            })}
          </ol>
        </section>
      ))}
    </div>
  );
}
