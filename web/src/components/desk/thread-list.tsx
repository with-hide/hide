import { ChevronDownIcon, ChevronRightIcon } from "lucide-react";
import type { DeskJob, DeskOutput, ThreadGroupKey } from "../../desk";
import { restingThreads, threadGroups } from "../../desk";
import { useState } from "react";
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
 * it (B37). Asleep and archived threads are counted at its foot, each count
 * opening its rows under it. `expanded` holds the Conductors shown open.
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
  const [resting, setResting] = useState<"sleeping" | "archived" | null>(null);
  const groups = threadGroups(jobs);
  const rest = restingThreads(jobs);
  const byPane = new Map(jobs.flatMap((job) => (job.pane_id === null ? [] : [[job.pane_id, job] as const])));
  if (groups.length === 0 && rest.sleeping.length === 0 && rest.archived.length === 0) return <p className="px-md py-lg text-caption text-muted-foreground" data-desk-threads-empty="true">{t("desk.threads.empty")}</p>;
  const row = (job: DeskJob) => <ThreadRow key={job.id} job={job} selected={selected === job.id} onOpen={onOpen} onMission={onMission} onOutput={onOutput} />;
  const foot = (key: "sleeping" | "archived", list: readonly DeskJob[]) => list.length === 0 ? null : (
    <Button variant="text" size="inline" className="gap-0 text-caption text-muted-foreground hover:text-foreground" aria-expanded={resting === key} data-desk-threads-resting={key} onClick={() => setResting(resting === key ? null : key)}>
      {t(key === "sleeping" ? "desk.threads.sleeping" : "desk.threads.archived", { count: list.length })}
      {resting === key ? <ChevronDownIcon aria-hidden="true" className="size-(--size-icon-sm)" /> : <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" />}
    </Button>
  );
  return (
    <div className="flex min-w-0 flex-col gap-sm px-xs py-xs" data-desk-threads={jobs.length}>
      {groups.map((group) => (
        <section key={group.key} className="flex min-w-0 flex-col" aria-label={t(GROUP[group.key], { count: group.jobs.length })} data-desk-thread-group={group.key}>
          <h3 className={cn("px-xs pb-xxs text-caption font-medium", group.key === "turn" ? "text-warning" : "text-muted-foreground")}>{t(GROUP[group.key], { count: group.jobs.length })}</h3>
          <ol className="flex min-w-0 flex-col">
            {group.jobs.map((job) => {
              const workers = job.workers.map((pane) => byPane.get(pane)).filter((one): one is DeskJob => one !== undefined);
              const open = expanded.has(job.id);
              return [
                <ThreadRow key={job.id} job={job} selected={selected === job.id} expanded={open} onOpen={onOpen} onMission={onMission} onOutput={onOutput} onToggle={workers.length ? () => onToggle(job.id) : undefined} />,
                ...(open ? workers.map((worker) => <ThreadRow key={worker.id} job={worker} selected={selected === worker.id} worker onOpen={onOpen} onMission={onMission} onOutput={onOutput} />) : []),
              ];
            })}
          </ol>
        </section>
      ))}
      {rest.sleeping.length || rest.archived.length ? (
        <section className="flex min-w-0 flex-col" data-desk-threads-foot="true">
          <p className="flex items-center gap-sm px-xs">{foot("sleeping", rest.sleeping)}{foot("archived", rest.archived)}</p>
          {resting ? <ol className="flex min-w-0 flex-col pt-xxs" data-desk-thread-group={resting}>{(resting === "sleeping" ? rest.sleeping : rest.archived).map(row)}</ol> : null}
        </section>
      ) : null}
    </div>
  );
}
