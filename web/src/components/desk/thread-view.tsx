import { CheckIcon, ChevronRightIcon, CircleAlertIcon, FilePenIcon, FileSearchIcon, GlobeIcon, LoaderCircleIcon, SquareTerminalIcon, WrenchIcon } from "lucide-react";
import type { ReactNode } from "react";
import type { AcpContentBlock, AcpPlanEntry, AcpToolKind, DeskJob, DeskMention, DeskOutput, ThreadHidePart, ThreadLine, ThreadReadFailure } from "../../desk";
import { useInterfaceTranslation } from "../../i18n/client";
import type { MessageKey } from "../../i18n/catalogs";
import { cn } from "../../lib/utils";
import { LetterTag } from "../letter-mark";
import { MentionChip } from "../mention-chip";
import { PlanChecklist } from "../plan-checklist";
import { StateBox, type StateBoxTone } from "../state-box";
import { StatusMark } from "../status-mark";
import { Button } from "../ui/button";
import { Hint } from "../ui/tooltip";
import { MentionText } from "./desk-chat";
import { DeskComposer, type ComposerAttachment } from "./desk-composer";
import { DeskMissionChip, jobLine } from "./desk-job-card";
import { DeskOutputs } from "./desk-outputs";
import { ThreadRow } from "./thread-list";

/** One step of the thread's path (B34): `Threads`, the Conductor, the worker; every step but the last opens its own place. */
export type ThreadPathStep = { label: string; onPress?: () => void };

/**
 * The thread's head inside the lane column's head (B25, B34, B35): its path
 * with the thread's title last, and `터미널로`, which goes to its Agent tab.
 */
export function ThreadHead({ path, onTerminal }: { path: readonly ThreadPathStep[]; onTerminal: () => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <span className="flex min-w-0 flex-1 items-center gap-xs">
      <nav className="flex min-w-0 flex-1 items-center gap-xxs" aria-label={t("desk.thread.path")} data-desk-thread-path={path.length}>
        {path.map((step, index) => {
          const last = index === path.length - 1;
          return (
            <span key={`${index}:${step.label}`} className={cn("flex items-center gap-xxs", last ? "min-w-0 flex-1" : "min-w-0 shrink")}>
              {last || !step.onPress
                ? <span className={cn("truncate", last ? "font-semibold text-foreground" : "text-muted-foreground")} aria-current={last ? "page" : undefined}>{step.label}</span>
                : <Button variant="text" size="inline" className="min-w-0 truncate text-muted-foreground" onClick={step.onPress}>{step.label}</Button>}
              {!last ? <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" /> : null}
            </span>
          );
        })}
      </nav>
      <Hint label={t("desk.thread.toTerminal")} shortcut="⌘J">
        <Button variant="ghost" size="sm" className="h-(--size-control-compact) gap-xxs px-xs text-caption font-normal text-subtle-foreground" data-desk-thread-terminal="true" onClick={onTerminal}>
          <SquareTerminalIcon />
          {t("desk.thread.toTerminal")}
        </Button>
      </Hint>
    </span>
  );
}

const NOW_TONE: Record<DeskJob["state"], StateBoxTone> = {
  queued: "quiet",
  starting: "quiet",
  working: "working",
  asking: "attention",
  done: "done",
  failed: "failed",
  cancelled: "quiet",
  closed: "quiet",
  moved_out: "quiet",
  archived: "quiet",
};

/** The 지금 box at the thread's top (D-61, B25): the session record's own box, in the job's state colour, with its line. */
export function ThreadNow({ job }: { job: DeskJob }) {
  const { t } = useInterfaceTranslation();
  return (
    <StateBox tone={NOW_TONE[job.state]} aria-label={t("desk.thread.now")} data-desk-thread-now={job.state}>
      <p className="flex min-w-0 items-baseline gap-sm text-caption">
        <span className="shrink-0 font-medium text-foreground">{t("desk.thread.now")}</span>
        <span className={cn("min-w-0 whitespace-normal break-keep [overflow-wrap:anywhere]", job.state === "asking" ? "text-warning" : job.state === "failed" ? "text-destructive" : "text-subtle-foreground")}>{jobLine(t, job)}</span>
      </p>
    </StateBox>
  );
}

const TOOL_ICON: Partial<Record<AcpToolKind, typeof WrenchIcon>> = {
  execute: SquareTerminalIcon,
  edit: FilePenIcon,
  delete: FilePenIcon,
  move: FilePenIcon,
  read: FileSearchIcon,
  search: FileSearchIcon,
  fetch: GlobeIcon,
};

/** An image the session saw or made, from its bytes; a cut image keeps only its place. */
function ThreadImage({ block }: { block: AcpContentBlock }) {
  if (block.type !== "image" || !block.data) return null;
  return <img src={`data:${block.mimeType};base64,${block.data}`} alt="" className="max-h-(--size-overview-graph-root-height) max-w-full rounded-sm border border-border object-contain" data-desk-thread-image="true" />;
}

/** A file's change: the lines before and after it, the unchanged ends trimmed to a line of context. */
export function DiffBlock({ path, oldText, newText }: { path: string; oldText?: string | null; newText: string }) {
  const before = oldText ? oldText.split("\n") : [];
  const after = newText.split("\n");
  let head = 0;
  while (head < before.length && head < after.length && before[head] === after[head]) head += 1;
  let tail = 0;
  while (tail < before.length - head && tail < after.length - head && before[before.length - 1 - tail] === after[after.length - 1 - tail]) tail += 1;
  const lines: { sign: "+" | "-" | " "; text: string }[] = [
    ...(head > 0 ? [{ sign: " " as const, text: before[head - 1]! }] : []),
    ...before.slice(head, before.length - tail).map((text) => ({ sign: "-" as const, text })),
    ...after.slice(head, after.length - tail).map((text) => ({ sign: "+" as const, text })),
    ...(tail > 0 ? [{ sign: " " as const, text: after[after.length - tail]! }] : []),
  ];
  return (
    <figure className="min-w-0 overflow-hidden rounded-sm border border-border" data-desk-thread-diff={path}>
      <figcaption className="truncate border-b border-border bg-muted px-sm py-xxs font-mono text-micro text-subtle-foreground">{path}</figcaption>
      <pre className="min-w-0 overflow-x-auto py-xxs font-mono text-micro">
        {lines.map((line, index) => (
          <div key={index} className={cn("px-sm", line.sign === "+" ? "bg-diff-added/15 text-foreground" : line.sign === "-" ? "bg-diff-removed/15 text-foreground" : "text-muted-foreground")} data-diff-sign={line.sign}>
            <span aria-hidden="true" className="mr-xs select-none text-muted-foreground">{line.sign}</span>{line.text}
          </div>
        ))}
      </pre>
    </figure>
  );
}

/** A tool call (B25): what kind and its title, its state while it runs or after it failed, the input it was given and what came out. */
function ToolLine({ line }: { line: Extract<ThreadLine, { kind: "tool" }> }) {
  const { t } = useInterfaceTranslation();
  const Icon = TOOL_ICON[line.toolKind] ?? WrenchIcon;
  const input = line.input && line.input !== line.title ? line.input : null;
  return (
    <div className="flex min-w-0 flex-col gap-xxs" data-desk-thread-line="tool" data-tool-status={line.status}>
      <p className="flex min-w-0 items-center gap-xs text-caption">
        <Icon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate font-mono text-subtle-foreground" title={line.title}>{line.title}</span>
        {line.status === "in_progress" || line.status === "pending" ? <span className="flex shrink-0 items-center gap-xxs text-micro text-agent-working"><LoaderCircleIcon aria-hidden="true" className="size-(--size-icon-sm) animate-spin" />{t("desk.thread.tool.running")}</span> : null}
        {line.status === "failed" ? <span className="shrink-0 text-micro text-destructive">{t("desk.thread.tool.failed")}</span> : null}
      </p>
      {input ? <pre aria-label={t("desk.thread.tool.input")} className="ml-(--size-icon) min-w-0 whitespace-pre-wrap break-all rounded-xs bg-muted px-xs py-xxs font-mono text-micro text-subtle-foreground">{input}</pre> : null}
      {line.diffs.map((diff) => <span key={diff.path} className="ml-(--size-icon) min-w-0"><DiffBlock path={diff.path} oldText={diff.oldText} newText={diff.newText} /></span>)}
      {line.output ? <pre aria-label={t("desk.thread.tool.output")} className="ml-(--size-icon) line-clamp-4 min-w-0 whitespace-pre-wrap break-all px-xs font-mono text-micro text-muted-foreground">{line.output}</pre> : null}
      {line.images.map((block, index) => <span key={index} className="ml-(--size-icon)"><ThreadImage block={block} /></span>)}
    </div>
  );
}

/** The agent's own todo list (ACP `plan`): each entry in its state, the current one in the working mark. */
function AcpPlan({ entries }: { entries: readonly AcpPlanEntry[] }) {
  return (
    <ol className="flex min-w-0 flex-col gap-xxs text-caption" data-desk-thread-line="plan">
      {entries.map((entry, index) => (
        <li key={index} className="flex min-w-0 items-start gap-xs">
          {entry.status === "completed" ? <CheckIcon aria-hidden="true" className="size-(--size-agent-mark) shrink-0 text-success" /> : <StatusMark mark={entry.status === "in_progress" ? "going" : "quiet"} />}
          <span className={cn("min-w-0", entry.status === "completed" ? "text-muted-foreground line-through" : entry.status === "in_progress" ? "font-medium text-foreground" : "text-subtle-foreground")}>{entry.content}</span>
        </li>
      ))}
    </ol>
  );
}

const PART_LABEL: Record<ThreadHidePart["kind"], MessageKey> = {
  plan: "desk.part.plan",
  spawn: "desk.part.spawn",
  letter: "desk.part.letter",
  view: "desk.part.view",
  mission: "desk.part.mission",
};

/** The command a part stands for, as the session typed it. */
const PART_COMMAND: Record<ThreadHidePart["kind"], string> = {
  plan: "hide plan",
  spawn: "hide agent spawn",
  letter: "hide request send",
  view: "hide browser open",
  mission: "hide mission",
};

export type ThreadPartActions = {
  onJob: (job: DeskJob) => void;
  onMission: (missionId: string) => void;
  onOutput: (output: DeskOutput) => void;
  onSession: (name: string) => void;
};

/**
 * A `hide` call drawn as the Hide part it made (D-72, B26): a plan's steps, a
 * new session's task row, a letter's line, an opened output, a mission's
 * chip, each in the shape the sidebar and the session record draw, under
 * the command's name. Pressing one goes to its place in Hide.
 */
function HidePartLine({ part, actions }: { part: ThreadHidePart; actions: ThreadPartActions }) {
  const { t } = useInterfaceTranslation();
  let body: ReactNode;
  switch (part.kind) {
    case "plan":
      body = <PlanChecklist plan={part.plan} />;
      break;
    case "spawn":
      body = <ol><ThreadRow job={part.job} selected={false} onOpen={actions.onJob} onMission={actions.onMission} /></ol>;
      break;
    case "letter":
      body = (
        <p className="flex min-w-0 items-baseline gap-xs text-caption text-subtle-foreground">
          <LetterTag who={<MentionChip kind="session" name={part.to} label={part.to} onPress={() => actions.onSession(part.to)} />} label={`${t("desk.part.letter")} ${part.to}`} />
          <span className="min-w-0 whitespace-normal break-keep [overflow-wrap:anywhere]">{part.body}</span>
        </p>
      );
      break;
    case "view":
      body = <DeskOutputs outputs={[part.output]} onOpen={actions.onOutput} />;
      break;
    case "mission":
      body = <p className="flex min-w-0 items-center gap-xs text-caption"><DeskMissionChip mission={part.mission} onOpen={actions.onMission} /><span className="min-w-0 truncate text-subtle-foreground">{part.mission.title}</span></p>;
      break;
  }
  return (
    <section className="flex min-w-0 flex-col gap-xxs rounded-md border border-border px-sm py-xs" aria-label={t(PART_LABEL[part.kind])} data-desk-thread-line="hide" data-desk-hide-part={part.kind}>
      <span className="text-micro text-muted-foreground"><span className="font-mono">{PART_COMMAND[part.kind]}</span> · {t(PART_LABEL[part.kind])}</span>
      {body}
    </section>
  );
}

/** One line of the thread (B25, B26). */
function Line({ line, mentions, actions }: { line: ThreadLine; mentions: readonly DeskMention[]; actions: ThreadPartActions }) {
  const { t } = useInterfaceTranslation();
  switch (line.kind) {
    case "user":
      return (
        <div className="flex min-w-0 flex-col items-end gap-xxs" data-desk-thread-line="user" data-origin={line.origin}>
          {line.origin === "letter" ? <LetterTag who={t("desk.part.letter")} label={t("desk.part.letter")} className="text-muted-foreground" /> : null}
          {line.text ? <p className="max-w-full whitespace-pre-wrap break-keep rounded-lg bg-secondary px-md py-xs text-body text-foreground [overflow-wrap:anywhere]">{line.text}</p> : null}
          {line.images.map((block, index) => <ThreadImage key={index} block={block} />)}
        </div>
      );
    case "agent":
      return <p className="min-w-0 whitespace-pre-wrap break-keep text-body text-foreground [overflow-wrap:anywhere]" data-desk-thread-line="agent"><MentionText text={line.text} mentions={mentions} onMention={(mention) => actions.onSession(mention.name)} /></p>;
    case "thought":
      return <p className="line-clamp-2 min-w-0 whitespace-pre-wrap text-caption italic text-muted-foreground" title={line.text} data-desk-thread-line="thought"><span className="not-italic">{t("desk.thread.thought")} · </span>{line.text}</p>;
    case "tool":
      return <ToolLine line={line} />;
    case "plan":
      return <AcpPlan entries={line.entries} />;
    case "hide":
      return <HidePartLine part={line.part} actions={actions} />;
  }
}

/**
 * A thread (D-61, B25-B27, B29): the 지금 box, the session's record drawn as
 * words, tool calls, diffs, images and plans with its `hide` calls as Hide's
 * own parts, its outputs, and the box that types into its terminal. A record
 * that cannot be read says why in one line and offers the terminal.
 */
export function ThreadView({ job, lines, failure = null, mentions = [], outputs, selectedOutput = null, composer, onTerminal, ...actions }: {
  job: DeskJob;
  lines: readonly ThreadLine[];
  failure?: ThreadReadFailure | null;
  /** The sessions and projects Hide knows by name, so an `@name` in the words is a chip. */
  mentions?: readonly DeskMention[];
  outputs: readonly DeskOutput[];
  selectedOutput?: string | null;
  composer: { value: string; attachments: readonly ComposerAttachment[]; focused?: boolean; onChange: (value: string) => void; onSend: () => void; onAttach: () => void; onRemoveAttachment: (id: string) => void };
  onTerminal: () => void;
} & ThreadPartActions) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-desk-thread={job.id}>
      <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-md overflow-y-auto px-md py-sm">
        <ThreadNow job={job} />
        {failure ? (
          <p role="alert" className="flex min-w-0 flex-wrap items-center gap-x-sm gap-y-xxs text-caption" data-desk-thread-failure={failure}>
            <CircleAlertIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-warning" />
            <span className="min-w-0 text-subtle-foreground">{t(failure === "no_record" ? "desk.thread.readFailed.no_record" : "desk.thread.readFailed.refused")}</span>
            <Button variant="text-link" size="inline" onClick={onTerminal}>{t("desk.thread.toTerminal")}</Button>
          </p>
        ) : (
          <ol className="flex min-w-0 flex-col gap-sm" data-desk-thread-lines={lines.length}>
            {lines.map((line) => <li key={line.id} className="min-w-0"><Line line={line} mentions={mentions} actions={actions} /></li>)}
          </ol>
        )}
        {outputs.length ? (
          <section className="flex min-w-0 flex-col gap-xs" aria-label={t("desk.outputs.title")} data-desk-thread-outputs={outputs.length}>
            <h3 className="text-caption font-medium text-muted-foreground">{t("desk.outputs.title")}</h3>
            <DeskOutputs outputs={outputs} selected={selectedOutput} onOpen={actions.onOutput} />
          </section>
        ) : null}
      </div>
      <div className="shrink-0 px-sm pb-sm">
        <DeskComposer
          value={composer.value}
          label={t("desk.thread.composerLabel", { title: job.title })}
          placeholder={t("desk.thread.placeholder")}
          attachments={composer.attachments}
          focused={composer.focused}
          onChange={composer.onChange}
          onSend={composer.onSend}
          onAttach={composer.onAttach}
          onRemoveAttachment={composer.onRemoveAttachment}
        />
      </div>
    </div>
  );
}
