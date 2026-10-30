import { CheckIcon, ChevronDownIcon, ChevronLeftIcon, ChevronRightIcon, CircleAlertIcon, FilePenIcon, FileSearchIcon, GlobeIcon, MailIcon, PinIcon, PlusIcon, SquareTerminalIcon, WrenchIcon, XIcon } from "lucide-react";
import { useLayoutEffect, useRef, useState, type ComponentPropsWithRef, type ReactNode } from "react";
import { Trans } from "react-i18next";
import type { AcpContentBlock, AcpPlanEntry, AcpToolKind, DeskJob, DeskMention, DeskOutput, DeskSessionRef, ThreadHidePart, ThreadLine, ThreadReadFailure, ThreadTurn } from "../../desk";
import { askedOnce, threadTurns } from "../../desk";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import { useRunningClock } from "../elapsed";
import { PlanChecklist } from "../plan-checklist";
import { PrMark } from "../pr-mark";
import { StatusMark } from "../status-mark";
import { Button } from "../ui/button";
import { Hint } from "../ui/tooltip";
import { MentionText } from "./desk-chat";
import { DeskComposer, type ComposerAttachment } from "./desk-composer";
import { DeskMissionChip, DeskPlanChip, JobLead, JobLine, jobStateWords, SessionChip } from "./desk-job-card";
import { DeskOutputs } from "./desk-outputs";

/** The Conductor a worker's thread sits under (B34): its title, and the way back to its thread. */
export type ThreadConductor = { title: string; onOpen: () => void };

/**
 * A thread's head, read as a pane's head (D-75, B25, B34, B35). The first
 * line is ‹, the session (its status, its agent, its title, after its
 * Conductor's for a worker), `터미널로` to its Agent tab, the pin and ✕; the
 * second is the sidebar row's second line, with the plan chip and the model
 * at its end. The thread's state is said here once.
 */
export function ThreadHead({ job, model = null, conductor = null, pinned, onBack, onTerminal, onPin, onClose }: {
  job: DeskJob;
  /** The model Hide launched the session with (`threadLaunchModel`); none when the launch chose none. */
  model?: string | null;
  conductor?: ThreadConductor | null;
  pinned: boolean;
  onBack: () => void;
  onTerminal: () => void;
  onPin: () => void;
  onClose: () => void;
}) {
  const { t } = useInterfaceTranslation();
  return (
    <header className="flex shrink-0 flex-col gap-xxs border-b border-border py-xs pl-xs pr-xs text-caption" data-desk-thread-head={job.state}>
      <div className="flex min-w-0 items-center gap-xs">
        <Hint label={conductor ? conductor.title : t("desk.thread.back")}>
          <Button variant="ghost" size="icon-sm" className="text-muted-foreground" data-desk-thread-back="true" onClick={conductor ? conductor.onOpen : onBack}><ChevronLeftIcon /></Button>
        </Hint>
        <JobLead job={job} />
        <nav className="flex min-w-0 flex-1 items-center gap-xxs" aria-label={t("desk.thread.path")} data-desk-thread-path={conductor ? 2 : 1}>
          {conductor ? (
            <>
              <Button variant="text" size="inline" className="min-w-0 max-w-1/2 shrink truncate text-muted-foreground" onClick={conductor.onOpen}>{conductor.title}</Button>
              <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
            </>
          ) : null}
          <span className="min-w-0 truncate text-body font-semibold text-foreground" aria-current="page">{job.title}</span>
        </nav>
        <Hint label={t("desk.thread.toTerminal")} shortcut="⌘J">
          <Button variant="ghost" size="sm" className="h-(--size-control-compact) gap-xxs px-xs text-caption font-normal text-subtle-foreground" data-desk-thread-terminal="true" onClick={onTerminal}>
            <SquareTerminalIcon />
            {t("desk.thread.toTerminal")}
          </Button>
        </Hint>
        <Hint label={pinned ? t("desk.lane.unpin") : t("desk.lane.pin")}>
          <Button variant="ghost" size="icon-sm" className={pinned ? "text-foreground" : "text-muted-foreground"} aria-pressed={pinned} data-lane-head-pin={pinned ? "on" : "off"} onClick={onPin}>
            <PinIcon className={cn(pinned && "fill-current")} />
          </Button>
        </Hint>
        <Hint label={t("desk.lane.close")}>
          <Button variant="ghost" size="icon-sm" className="text-muted-foreground" onClick={onClose}><XIcon /></Button>
        </Hint>
      </div>
      <div className="flex min-w-0 items-center gap-xs pl-(--size-control-sm)">
        <JobLine job={job} className="flex-1" />
        {job.plan ? <DeskPlanChip plan={job.plan} review={job.ask === "plan_review"} /> : null}
        {model ? <span className="shrink-0 text-micro text-muted-foreground" data-desk-thread-model="true">{model}</span> : null}
      </div>
    </header>
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
  const { t } = useInterfaceTranslation();
  if (block.type !== "image" || !block.data) return null;
  return <img src={`data:${block.mimeType};base64,${block.data}`} alt={t("desk.thread.image")} className="max-h-(--size-overview-graph-root-height) max-w-full rounded-sm border border-border object-contain" data-desk-thread-image="true" />;
}

type DiffLine = { sign: "+" | "-" | " "; text: string };

/** A file's change as lines: the unchanged ends trimmed to one line of context each. */
function diffLines(oldText: string | null | undefined, newText: string): DiffLine[] {
  const before = oldText ? oldText.split("\n") : [];
  const after = newText.split("\n");
  let head = 0;
  while (head < before.length && head < after.length && before[head] === after[head]) head += 1;
  let tail = 0;
  while (tail < before.length - head && tail < after.length - head && before[before.length - 1 - tail] === after[after.length - 1 - tail]) tail += 1;
  return [
    ...(head > 0 ? [{ sign: " " as const, text: before[head - 1]! }] : []),
    ...before.slice(head, before.length - tail).map((text) => ({ sign: "-" as const, text })),
    ...after.slice(head, after.length - tail).map((text) => ({ sign: "+" as const, text })),
    ...(tail > 0 ? [{ sign: " " as const, text: after[after.length - tail]! }] : []),
  ];
}

/** A file's change under its tool line (B25): the changed lines on the diff grounds, with one line of context. */
function DiffBlock({ path, lines }: { path: string; lines: readonly DiffLine[] }) {
  return (
    <pre className="min-w-0 overflow-x-auto rounded-sm py-xxs font-mono text-micro" data-desk-thread-diff={path}>
      {lines.map((line, index) => (
        <div key={index} className={cn("px-sm", line.sign === "+" ? "bg-diff-added/15 text-foreground" : line.sign === "-" ? "bg-diff-removed/15 text-foreground" : "text-muted-foreground")} data-diff-sign={line.sign}>
          <span aria-hidden="true" className="mr-xs select-none text-muted-foreground">{line.sign}</span>{line.text}
        </div>
      ))}
    </pre>
  );
}

/**
 * A tool call (B25): its kind's icon and its title, a command as the
 * command itself, `+n −m` after an edit, `실행 중` in words while it runs
 * (a ring is a session's state, D-75) and `실패` after it failed; then its
 * diff and what came out, cut at four lines.
 */
function ToolLine({ line }: { line: Extract<ThreadLine, { kind: "tool" }> }) {
  const { t } = useInterfaceTranslation();
  const Icon = TOOL_ICON[line.toolKind] ?? WrenchIcon;
  const diffs = line.diffs.map((diff) => ({ path: diff.path, lines: diffLines(diff.oldText, diff.newText) }));
  const added = diffs.reduce((sum, diff) => sum + diff.lines.filter((one) => one.sign === "+").length, 0);
  const removed = diffs.reduce((sum, diff) => sum + diff.lines.filter((one) => one.sign === "-").length, 0);
  const title = line.toolKind === "execute" && line.input ? line.input : line.title;
  const running = line.status === "in_progress" || line.status === "pending";
  const clock = useRunningClock(running && line.toolKind === "execute" ? line.startedAtMs : null);
  return (
    <div className="flex min-w-0 flex-col gap-xxs" data-desk-thread-line="tool" data-tool-status={line.status}>
      <p className="flex min-w-0 items-center gap-xs text-caption">
        <Icon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
        <span className="min-w-0 truncate font-mono text-subtle-foreground" title={title}>{title}</span>
        {diffs.length ? <span className="shrink-0 font-mono text-micro"><span className="text-success">+{added}</span> <span className="text-destructive">−{removed}</span></span> : null}
        <span className="flex-1" />
        {running ? <span className="shrink-0 text-micro text-muted-foreground" data-desk-tool-clock={clock ?? undefined}>{clock ? `${t("desk.thread.tool.running")} ${clock}` : t("desk.thread.tool.running")}</span> : null}
        {line.status === "failed" ? <span className="shrink-0 text-micro text-destructive">{t("desk.thread.tool.failed")}</span> : null}
      </p>
      {diffs.map((diff, index) => <span key={`${index}:${diff.path}`} className="min-w-0"><DiffBlock path={diff.path} lines={diff.lines} /></span>)}
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

export type ThreadPartActions = {
  onMission: (missionId: string) => void;
  onOutput: (output: DeskOutput) => void;
  /** A session a chip or an `@name` names: its pane when Hide knows it, else its name alone. */
  onSession: (session: DeskSessionRef) => void;
};

/**
 * A line a Hide part takes in the thread: an icon and its words, with no box.
 * It takes the props and the ref a hint's trigger hands it, so the command
 * behind it shows on hover.
 */
function PartLine({ icon, kind, children, className, ...rest }: { icon: ReactNode; kind: ThreadHidePart["kind"] | "received"; children: ReactNode } & ComponentPropsWithRef<"p">) {
  return (
    <p {...rest} className={cn("flex min-w-0 items-center gap-xs text-caption text-muted-foreground", className)} data-desk-thread-line="hide" data-desk-hide-part={kind}>
      {icon}
      {children}
    </p>
  );
}

const PART_ICON = "size-(--size-icon-sm) shrink-0";

/**
 * A `hide` call drawn as the Hide part it made (D-72, D-75, B26), with no
 * box and no command; the command the session typed is the hover hint. A
 * letter reads `✉ [session]에게 보냄 · first line`, a new session
 * `＋ 새 스레드에 맡김 [session]`, an opened output as its picture or row, a
 * mission as its chip, a PR as its mark, a plan as its steps. Pressing a
 * chip goes to its place in Hide.
 */
function HidePartLine({ part, command, actions }: { part: ThreadHidePart; command: string | null; actions: ThreadPartActions }) {
  const { t, i18n } = useInterfaceTranslation();
  let body: ReactNode;
  switch (part.kind) {
    case "plan":
      body = <div data-desk-thread-line="hide" data-desk-hide-part="plan"><PlanChecklist plan={part.plan} /></div>;
      break;
    case "spawn":
      body = (
        <PartLine kind="spawn" icon={<PlusIcon aria-hidden="true" className={PART_ICON} />}>
          <span className="shrink-0">{t("desk.part.spawned")}</span>
          <SessionChip session={part.session} onPress={() => actions.onSession(part.session)} />
        </PartLine>
      );
      break;
    case "letter":
      body = (
        <PartLine kind="letter" icon={<MailIcon aria-hidden="true" className={PART_ICON} />}>
          <span className="shrink-0" data-desk-letter={part.direction}>
            <Trans i18n={i18n} t={t} i18nKey={part.direction === "sent" ? "desk.letter.sent" : "desk.letter.received"} components={{ who: <SessionChip session={part.peer} className="mr-xxs" onPress={() => actions.onSession(part.peer)} /> }} />
          </span>
          {part.body ? <><span aria-hidden="true">·</span><span className="min-w-0 truncate text-subtle-foreground">{part.body}</span></> : null}
        </PartLine>
      );
      break;
    case "view":
      body = <div data-desk-thread-line="hide" data-desk-hide-part="view"><DeskOutputs outputs={[part.output]} onOpen={actions.onOutput} /></div>;
      break;
    case "mission":
      body = (
        <PartLine kind="mission" icon={null}>
          <DeskMissionChip mission={part.mission} onOpen={actions.onMission} />
          <span className="min-w-0 truncate text-subtle-foreground">{part.mission.title}</span>
        </PartLine>
      );
      break;
    case "pr":
      body = <PartLine kind="pr" icon={null}><PrMark state={part.state} number={part.number} /></PartLine>;
      break;
  }
  return command ? <Hint label={command} reveals contentClassName="font-mono">{body}</Hint> : body;
}

/** One line of a turn's work (B25, B26). */
function WorkLine({ line, mentions, actions }: { line: ThreadLine; mentions: readonly DeskMention[]; actions: ThreadPartActions }) {
  const { t } = useInterfaceTranslation();
  switch (line.kind) {
    case "user":
      return null;
    case "agent":
      return <AgentWords text={line.text} mentions={mentions} actions={actions} />;
    case "thought":
      return <p className="line-clamp-2 min-w-0 whitespace-pre-wrap text-caption text-muted-foreground" title={line.text} data-desk-thread-line="thought">{t("desk.thread.thought")} · {line.text}</p>;
    case "tool":
      return <ToolLine line={line} />;
    case "plan":
      return <AcpPlan entries={line.entries} />;
    case "hide":
      return <HidePartLine part={line.part} command={line.command} actions={actions} />;
  }
}

function AgentWords({ text, mentions, actions }: { text: string; mentions: readonly DeskMention[]; actions: ThreadPartActions }) {
  return <p className="min-w-0 whitespace-pre-wrap break-keep text-body leading-relaxed text-foreground [overflow-wrap:anywhere]" data-desk-thread-line="agent"><MentionText text={text} mentions={mentions} onMention={(mention) => actions.onSession({ name: mention.name, mark: mention.mark ?? null, agent_kind: mention.agent_kind ?? null })} /></p>;
}

/**
 * A letter the session got, as a line and not a bubble (D-75): `✉ [sender]에게서
 * 받음 · first line`, the whole letter on hover; a letter whose record names
 * no sender reads `✉ 편지 · first line`.
 */
function ReceivedLine({ line, actions }: { line: Extract<ThreadLine, { kind: "user" }>; actions: ThreadPartActions }) {
  const { t, i18n } = useInterfaceTranslation();
  const peer = line.peer;
  const first = line.text.trim().split("\n", 1)[0] ?? "";
  return (
    <Hint label={line.text.trim()} reveals>
      <PartLine kind="received" icon={<MailIcon aria-hidden="true" className={PART_ICON} />} data-origin="letter">
        <span className="shrink-0" data-desk-letter="received">
          {peer ? <Trans i18n={i18n} t={t} i18nKey="desk.letter.received" components={{ who: <SessionChip session={peer} className="mr-xxs" onPress={() => actions.onSession(peer)} /> }} /> : t("desk.part.letter")}
        </span>
        <span aria-hidden="true">·</span>
        <span className="min-w-0 truncate text-subtle-foreground">{first}</span>
      </PartLine>
    </Hint>
  );
}

/** The operator's words, the one thing drawn as a bubble (D-75); the first one carries `Desk가 맡김` when Desk handed the task. */
function UserLine({ line, handed, actions }: { line: Extract<ThreadLine, { kind: "user" }>; handed: boolean; actions: ThreadPartActions }) {
  const { t } = useInterfaceTranslation();
  if (line.origin === "letter") return <ReceivedLine line={line} actions={actions} />;
  return (
    <div className="flex min-w-0 flex-col items-end gap-xxs" data-desk-thread-line="user" data-origin={line.origin}>
      {handed ? <span className="text-micro text-muted-foreground" data-desk-thread-handed="true">{t("desk.thread.handedByDesk")}</span> : null}
      {line.text ? <p className="max-w-(--size-pr-popover) whitespace-pre-wrap break-keep rounded-xl bg-secondary px-md py-xs text-body leading-relaxed text-foreground [overflow-wrap:anywhere]">{line.text}</p> : null}
      {line.images.map((block, index) => <ThreadImage key={index} block={block} />)}
    </div>
  );
}

/**
 * One turn (B25, D-75): the operator's words, the work folded into one line
 * `작업 · 파일 n · 명령 m ›`, the answer. The turn still running shows its
 * work open, along a rail. A part that leads somewhere (a new session, a
 * letter, an output, a mission, a PR) stays in view while the work is folded.
 */
function Turn({ turn, live, handed, mentions, actions }: { turn: ThreadTurn; live: boolean; handed: boolean; mentions: readonly DeskMention[]; actions: ThreadPartActions }) {
  const { t } = useInterfaceTranslation();
  const [toggled, setToggled] = useState(false);
  const open = live !== toggled;
  const counts = [turn.files ? t("desk.turn.files", { count: turn.files }) : null, turn.commands ? t("desk.turn.commands", { count: turn.commands }) : null].filter(Boolean);
  const Chevron = open ? ChevronDownIcon : ChevronRightIcon;
  return (
    <li className="flex min-w-0 flex-col gap-sm" data-desk-thread-turn={open ? "open" : "folded"}>
      {turn.user ? <UserLine line={turn.user} handed={handed} actions={actions} /> : null}
      {turn.work.length ? (
        <Button variant="text" size="inline" className="self-start gap-xxs text-caption text-muted-foreground" aria-expanded={open} data-desk-turn-toggle={open ? "open" : "folded"} onClick={() => setToggled(!toggled)}>
          {[live ? t("desk.turn.working") : t("desk.turn.work"), ...counts].join(" · ")}
          <Chevron aria-hidden="true" className="size-(--size-icon-sm)" />
        </Button>
      ) : null}
      {open && turn.work.length ? (
        <ol className="ml-xxs flex min-w-0 flex-col gap-xs border-l border-border pl-sm" data-desk-thread-work={turn.work.length}>
          {turn.work.map((line) => <li key={line.id} className="min-w-0"><WorkLine line={line} mentions={mentions} actions={actions} /></li>)}
        </ol>
      ) : null}
      {turn.answer.map((line) => <AgentWords key={line.id} text={line.text} mentions={mentions} actions={actions} />)}
      {!open && turn.parts.length ? <FoldedParts parts={turn.parts} actions={actions} /> : null}
    </li>
  );
}

/** What a folded turn keeps in view (B26): its lines that lead somewhere, then the outputs it opened as one row of pictures. */
function FoldedParts({ parts, actions }: { parts: ThreadTurn["parts"]; actions: ThreadPartActions }) {
  const outputs = parts.flatMap((line) => (line.part.kind === "view" ? [line.part.output] : []));
  const rest = parts.filter((line) => line.part.kind !== "view");
  return (
    <div className="flex min-w-0 flex-col gap-xs" data-desk-thread-parts={parts.length}>
      {rest.map((line) => <HidePartLine key={line.id} part={line.part} command={line.command} actions={actions} />)}
      {outputs.length ? <DeskOutputs outputs={outputs} onOpen={actions.onOutput} /> : null}
    </div>
  );
}

/**
 * The wait at the conversation's foot (B20, B25, D-75), the one place a
 * question sits: the asking mark and what it asks; a permission shows the
 * command on its grey block.
 */
function ThreadAsk({ job }: { job: DeskJob }) {
  const { t } = useInterfaceTranslation();
  const permission = job.ask === "approval";
  return (
    <section className="flex min-w-0 flex-col gap-xs" aria-label={jobStateWords(t, job)} data-desk-thread-ask={job.ask ?? "question"}>
      <p className="flex min-w-0 items-start gap-xs text-body text-foreground">
        <StatusMark mark="turn" read={job.mark_read} />
        <span className="min-w-0 whitespace-normal break-keep [overflow-wrap:anywhere]">{permission ? t("desk.thread.askPermission") : (job.question ?? jobStateWords(t, job))}</span>
      </p>
      {permission && job.question ? <pre className="ml-(--size-agent-mark) min-w-0 whitespace-pre-wrap break-all rounded-sm bg-secondary px-sm py-xs font-mono text-caption text-foreground">{job.question}</pre> : null}
    </section>
  );
}

/** How near an end of the conversation still counts as at it, in pixels. */
const AT_END = 24;

/**
 * The conversation's scrolling (B25): while the operator is at its end, what
 * the session writes next is brought into view; a page read in above keeps
 * the lines they are reading where they were; reaching the top asks for the
 * page before.
 */
function useThreadScroll(lines: readonly ThreadLine[], onReachTop?: () => void) {
  const ref = useRef<HTMLDivElement>(null);
  const atEnd = useRef(true);
  const before = useRef<{ first: string | null; height: number }>({ first: null, height: 0 });
  useLayoutEffect(() => {
    const box = ref.current;
    if (!box) return;
    // A message line's id is its place, which a page read in above moves, so the first line is known by its words too.
    const head = lines[0];
    const first = head ? `${head.kind}:${head.id}:${"text" in head ? head.text.slice(0, 80) : ""}` : null;
    if (atEnd.current) box.scrollTop = box.scrollHeight;
    else if (before.current.first !== null && first !== before.current.first) box.scrollTop += box.scrollHeight - before.current.height;
    before.current = { first, height: box.scrollHeight };
  }, [lines]);
  const onScroll = () => {
    const box = ref.current;
    if (!box) return;
    atEnd.current = box.scrollHeight - box.scrollTop - box.clientHeight < AT_END;
    before.current.height = box.scrollHeight;
    if (box.scrollTop < AT_END) onReachTop?.();
  };
  return { ref, onScroll };
}

/**
 * A thread (D-61, D-75, B25-B27, B29): the session's record as turns, its
 * question at the foot, its outputs after it, and the box that types into
 * its terminal. Its state is in the head, said once. A record that cannot be
 * read says why in one line and offers the terminal; one being read, or with
 * nothing in it yet, says so in one quiet line.
 */
export function ThreadView({ job, lines, reading = false, failure = null, mentions = [], startedByDesk = false, outputs, selectedOutput = null, composer, onTerminal, onReachTop, ...actions }: {
  job: DeskJob;
  lines: readonly ThreadLine[];
  /** The record is still being read: `lines` is not the thread yet. */
  reading?: boolean;
  failure?: ThreadReadFailure | null;
  /** The sessions and projects Hide knows by name, so an `@name` in the words is a chip. */
  mentions?: readonly DeskMention[];
  /** Desk handed the thread its task: the first message says so. */
  startedByDesk?: boolean;
  outputs: readonly DeskOutput[];
  selectedOutput?: string | null;
  composer: { value: string; attachments: readonly ComposerAttachment[]; focused?: boolean; held?: boolean; notice?: string | null; onChange: (value: string) => void; onSend: () => void; onAttach: () => void; onRemoveAttachment: (id: string) => void };
  onTerminal: () => void;
  /** The conversation was scrolled to its top: the page before it can be read. */
  onReachTop?: () => void;
} & ThreadPartActions) {
  const { t } = useInterfaceTranslation();
  const scroller = useThreadScroll(lines, onReachTop);
  // An asking job's question is said once, in the wait at the foot (D-75).
  const turns = job.state === "asking" && job.ask === "question" ? askedOnce(threadTurns(lines), job.question) : threadTurns(lines);
  const running = job.state === "working" || job.state === "starting";
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-desk-thread={job.id}>
      <div ref={scroller.ref} onScroll={scroller.onScroll} data-desk-thread-conversation="true" className="flex min-h-0 min-w-0 flex-1 flex-col gap-md overflow-y-auto px-md py-md">
        {failure ? (
          <p role="alert" className="flex min-w-0 flex-wrap items-center gap-x-sm gap-y-xxs text-caption" data-desk-thread-failure={failure}>
            <CircleAlertIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-warning" />
            <span className="min-w-0 text-subtle-foreground">{t(failure === "no_record" ? "desk.thread.readFailed.no_record" : "desk.thread.readFailed.refused")}</span>
            <Button variant="text-link" size="inline" onClick={onTerminal}>{t("desk.thread.toTerminal")}</Button>
          </p>
        ) : reading ? (
          <p role="status" className="animate-pulse text-caption text-muted-foreground" data-desk-thread-reading="true">{t("desk.thread.reading")}</p>
        ) : lines.length === 0 ? (
          <p className="text-caption text-muted-foreground" data-desk-thread-empty="true">{t("desk.thread.empty")}</p>
        ) : (
          <ol className="flex min-w-0 flex-col gap-lg" data-desk-thread-lines={lines.length}>
            {turns.map((turn, index) => <Turn key={turn.key} turn={turn} live={running && index === turns.length - 1} handed={startedByDesk && index === 0} mentions={mentions} actions={actions} />)}
          </ol>
        )}
        {job.state === "asking" ? <ThreadAsk job={job} /> : null}
        {outputs.length ? (
          <section className="flex min-w-0 flex-col gap-xs" aria-label={t("desk.outputs.title")} data-desk-thread-outputs={outputs.length}>
            <h3 className="text-caption text-muted-foreground">{t("desk.outputs.title")}</h3>
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
          held={composer.held}
          notice={composer.notice}
          onChange={composer.onChange}
          onSend={composer.onSend}
          onAttach={composer.onAttach}
          onRemoveAttachment={composer.onRemoveAttachment}
        />
      </div>
    </div>
  );
}
