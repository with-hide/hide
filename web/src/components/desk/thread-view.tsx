import { CheckIcon, ChevronDownIcon, ChevronRightIcon, CircleAlertIcon, FilePenIcon, FileSearchIcon, GlobeIcon, MailIcon, PlusIcon, SquareTerminalIcon, WrenchIcon } from "lucide-react";
import { pullLabel } from "../../codeHost";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ComponentPropsWithRef, type ReactNode } from "react";
import { Trans } from "react-i18next";
import type { AcpContentBlock, AcpPlanEntry, AcpToolKind, DeskJob, DeskMention, DeskOutput, DeskSessionRef, ThreadHidePart, ThreadLine, ThreadReadFailure, ThreadTurn } from "../../desk";
import { askedIn, namedMention, threadPhase, threadTurns } from "../../desk";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import { MarkdownText, type MarkdownChip } from "../../MarkdownText";
import { markdownBlockHolding } from "../../markdownPlain";
import { CopyableMessage } from "../copy-words";
import type { CopyText } from "../human-turn-copy";
import type { AskLine } from "../open-asks";
import { useRunningClock } from "../elapsed";
import { PlanChecklist } from "../plan-checklist";
import { PrMark } from "../pr-mark";
import { StatusMark } from "../status-mark";
import { Button } from "../ui/button";
import { Hint } from "../ui/tooltip";
import { DeskComposer, SentAttachments } from "./desk-composer";
import { sentWords, type ComposerTray } from "../../deskComposer";
import { DeskMissionChip } from "../mission-chip";
import { WorkingLine } from "./working-line";
import { SessionChip, useSessionChipBridge } from "./session-chip";
import { DeskOutputs } from "./desk-outputs";
import { answerPlace, type AnswerPlace, type FoundAnswerPath } from "../../answerPaths";
import { useShellStore } from "../../store";
import { useShallow } from "zustand/react/shallow";
import { useFilePathChips } from "./file-path-chip";
import { ThreadAsks } from "./thread-asks";

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
  /**
   * An output a line names, placed where it is read: the card's own when it
   * lists it, else the file on the machine the session runs on (B27). What
   * the thread draws and what `onOutput` opens are both this answer.
   */
  outputOf: (output: DeskOutput) => DeskOutput;
  onOutput: (output: DeskOutput) => void;
  /** A session a chip or an `@name` names: its pane when Hide knows it, else its name alone. */
  onSession: (session: DeskSessionRef) => void;
  /** Puts a message's or a code block's words on the clipboard (Doop E2). */
  onCopy: CopyText;
  /** A file path the session's words name, pressed (feedback 2 frame M [2]); without it paths stay words. */
  onFile?: (file: FoundAnswerPath) => void;
};

/** How the thread draws the agent's words: the `@name`s as chips, and the clipboard its `복사` writes to; both kept stable between renders. */
/** `place` is where the session's relative paths are read (null when its file paths stay words), `file` what a file chip's press does. */
type Words = { chips: readonly MarkdownChip[]; copy: CopyText; place: AnswerPlace | null; file: (file: FoundAnswerPath) => void };

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
      body = <div data-desk-thread-line="hide" data-desk-hide-part="view"><DeskOutputs outputs={[actions.outputOf(part.output)]} onOpen={actions.onOutput} /></div>;
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
      body = <PartLine kind="pr" icon={null}><PrMark state={part.state} label={pullLabel(part)} /></PartLine>;
      break;
  }
  return command ? <Hint label={command} reveals contentClassName="font-mono">{body}</Hint> : body;
}

/** One line of a turn's work (B25, B26). */
function WorkLine({ line, words, actions }: { line: ThreadLine; words: Words; actions: ThreadPartActions }) {
  const { t } = useInterfaceTranslation();
  switch (line.kind) {
    case "user":
      return null;
    case "agent":
      return <AgentWords line={line} words={words} lit={false} live={false} />;
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

/**
 * The agent's words as Desk's answers read (Doop E2): Markdown in the
 * answer's type, an `@name` as its session's chip, `복사` of the original
 * words on hover and on every code block. `lit` lights the message a
 * question Hide found was asked in, for a moment after the question line
 * brought it into view.
 */
function AgentWords({ line, words, lit, live }: { line: Extract<ThreadLine, { kind: "agent" }>; words: Words; lit: boolean; live: boolean }) {
  // The file paths it names become chips once the turn has said them (feedback 2 frame M [2]).
  const files = useFilePathChips(line.text, words.place, live, words.file);
  const chips = useMemo(() => (files.length ? [...words.chips, ...files] : words.chips), [words.chips, files]);
  return (
    <CopyableMessage text={line.text} copy={words.copy} className={cn("-mx-xs rounded-md px-xs transition-colors", lit && "bg-open-ask/10 ring-1 ring-open-ask")} data-desk-thread-line="agent" data-desk-thread-message={line.id} data-desk-thread-lit={lit ? "true" : undefined}>
      <MarkdownText text={line.text} size="chat" chips={chips} copyCode={words.copy} />
    </CopyableMessage>
  );
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
function UserLine({ line, handed, copy, actions }: { line: Extract<ThreadLine, { kind: "user" }>; handed: boolean; copy: CopyText; actions: ThreadPartActions }) {
  const { t } = useInterfaceTranslation();
  // The files a composer sent stand over the words, read from the quoted paths it typed before them (B41).
  const sent = useMemo(() => sentWords(line.text), [line.text]);
  if (line.origin === "letter") return <ReceivedLine line={line} actions={actions} />;
  // Whatever an agent records when its turn was cut short is one quiet word, never the operator's bubble (D-75).
  if (line.origin === "interrupted") return <p className="text-micro text-muted-foreground" data-desk-thread-line="interrupted">{t("desk.thread.interrupted")}</p>;
  return (
    <div className="flex min-w-0 flex-col items-end gap-xxs" data-desk-thread-line="user" data-origin={line.origin}>
      {handed ? <span className="text-micro text-muted-foreground" data-desk-thread-handed="true">{t("desk.thread.handedByDesk")}</span> : null}
      <SentAttachments files={sent.files} />
      {sent.words ? (
        <CopyableMessage text={sent.words} copy={copy} align="end">
          <p data-desk-thread-bubble="true" className="max-w-(--size-pr-popover) whitespace-pre-wrap break-keep rounded-xl bg-secondary px-md py-xs text-subhead leading-relaxed text-foreground [overflow-wrap:anywhere]">{sent.words}</p>
        </CopyableMessage>
      ) : null}
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
function Turn({ turn, live, handed, words, lit, actions }: { turn: ThreadTurn; live: boolean; handed: boolean; words: Words; lit: string | null; actions: ThreadPartActions }) {
  const { t } = useInterfaceTranslation();
  const [toggled, setToggled] = useState(false);
  const open = live !== toggled;
  const counts = [turn.files ? t("desk.turn.files", { count: turn.files }) : null, turn.commands ? t("desk.turn.commands", { count: turn.commands }) : null].filter(Boolean);
  const Chevron = open ? ChevronDownIcon : ChevronRightIcon;
  return (
    <li className="flex min-w-0 flex-col gap-sm" data-desk-thread-turn={open ? "open" : "folded"}>
      {turn.user ? <UserLine line={turn.user} handed={handed} copy={words.copy} actions={actions} /> : null}
      {turn.work.length ? (
        <Button variant="text" size="inline" className="self-start gap-xxs text-caption text-muted-foreground" aria-expanded={open} data-desk-turn-toggle={open ? "open" : "folded"} onClick={() => setToggled(!toggled)}>
          {[t("desk.turn.work"), ...counts].join(" · ")}
          <Chevron aria-hidden="true" className="size-(--size-icon-sm)" />
        </Button>
      ) : null}
      {open && turn.work.length ? (
        <ol className="ml-xxs flex min-w-0 flex-col gap-xs border-l border-border pl-sm" data-desk-thread-work={turn.work.length}>
          {workRows(turn.work).map((row) =>
            Array.isArray(row) ? (
              <li key={row[0]!.id} className="min-w-0">
                <div data-desk-thread-line="hide" data-desk-hide-part="view"><DeskOutputs outputs={row.map((line) => actions.outputOf(line.part.output))} onOpen={actions.onOutput} /></div>
              </li>
            ) : (
              <li key={row.id} className="min-w-0"><WorkLine line={row} words={words} actions={actions} /></li>
            ))}
        </ol>
      ) : null}
      {turn.answer.map((line) => <AgentWords key={line.id} line={line} words={words} lit={lit === line.id} live={live} />)}
      {!open && turn.parts.length ? <FoldedParts parts={turn.parts} actions={actions} /> : null}
    </li>
  );
}

type ViewLine = Extract<ThreadLine, { kind: "hide" }> & { part: Extract<ThreadHidePart, { kind: "view" }> };

/** A turn's work as its open fold draws it: views opened one after another share one row of tiles, as the folded turn's do (feedback 2, frame M 나). */
function workRows(work: readonly ThreadLine[]): (ThreadLine | ViewLine[])[] {
  const rows: (ThreadLine | ViewLine[])[] = [];
  for (const line of work) {
    const last = rows.at(-1);
    if (!isViewLine(line)) rows.push(line);
    else if (Array.isArray(last)) last.push(line);
    else rows.push([line]);
  }
  return rows;
}

function isViewLine(line: ThreadLine): line is ViewLine {
  return line.kind === "hide" && line.part.kind === "view";
}

/** What a folded turn keeps in view (B26): its lines that lead somewhere, then the outputs it opened as one row of pictures. */
function FoldedParts({ parts, actions }: { parts: ThreadTurn["parts"]; actions: ThreadPartActions }) {
  const outputs = parts.flatMap((line) => (line.part.kind === "view" ? [actions.outputOf(line.part.output)] : []));
  const rest = parts.filter((line) => line.part.kind !== "view");
  return (
    <div className="flex min-w-0 flex-col gap-xs" data-desk-thread-parts={parts.length}>
      {rest.map((line) => <HidePartLine key={line.id} part={line.part} command={line.command} actions={actions} />)}
      {outputs.length ? <DeskOutputs outputs={outputs} onOpen={actions.onOutput} /> : null}
    </div>
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

/** How long the message a question was asked in stays lit after the question line brought it into view (Doop B4). */
const LIT_MS = 2_000;

/**
 * The thread's `@name`s as chips and its clipboard, kept stable while the
 * names stay the same, so a message's Markdown is drawn again only when its
 * words change.
 */
/** The defaults a caller without mentions or questions gets, one array each, so nothing reads them as changed. */
export const NO_MENTIONS: readonly DeskMention[] = [];
const NO_ASKS: readonly AskLine[] = [];

function useWords(mentions: readonly DeskMention[], onSession: ThreadPartActions["onSession"], onCopy: CopyText, place: AnswerPlace | null, onFile: ThreadPartActions["onFile"]): Words {
  const latest = useRef({ onSession, onCopy, onFile });
  latest.current = { onSession, onCopy, onFile };
  const bridge = useSessionChipBridge();
  const chips = useMemo<MarkdownChip[]>(() => mentions.map((mention) => {
    const session: DeskSessionRef = { name: mention.name, mark: mention.mark ?? null, agent_kind: mention.agent_kind ?? null };
    return { token: namedMention(mention), node: bridge(<SessionChip session={session} onPress={() => latest.current.onSession(session)} />) };
  }), [mentions, bridge]);
  const copy = useCallback<CopyText>((text, what) => latest.current.onCopy(text, what), []);
  const file = useCallback((found: FoundAnswerPath) => latest.current.onFile?.(found), []);
  return useMemo(() => ({ chips, copy, place: onFile ? place : null, file }), [chips, copy, place, onFile, file]);
}

/**
 * A thread (D-61, D-75, B25-B27, B29): the session's record as turns, what
 * the agent's own screen asks at its end (a choice card answered in place),
 * the questions Hide found in one line above the input box (Doop B4, N 2), and
 * the box that types into its terminal. Its state is in the head, said once.
 * A record that cannot be read says why in one line and offers the
 * terminal; one being read, or with nothing in it yet, says so in one quiet
 * line. A session asleep with no card any more (its sleep closed its pane,
 * B43) keeps its record above the foot that wakes it.
 */
export function ThreadView({ job, lines, reading = false, failure = null, mentions = NO_MENTIONS, startedByDesk = false, composer, ask = null, asks = NO_ASKS, onAsk, foot = null, workingSince = null, onTerminal, onReachTop, ...actions }: {
  job: DeskJob | null;
  /** When the session's running turn started (its row's last prompt), for the working line's time. */
  workingSince?: number | null;
  lines: readonly ThreadLine[];
  /** The record is still being read: `lines` is not the thread yet. */
  reading?: boolean;
  failure?: ThreadReadFailure | null;
  /** The sessions and projects Hide knows by name, so an `@name` in the words is a chip. */
  mentions?: readonly DeskMention[];
  /** Desk handed the thread its task: the first message says so. */
  startedByDesk?: boolean;
  composer: { value: string; tray: ComposerTray; focused?: boolean; held?: boolean; notice?: string | null; onChange: (value: string) => void; onSend: () => void };
  /** What the agent's own screen asks, at the conversation's end, answered in place (the choice card, PRD desk-2-ui B14). */
  ask?: ReactNode;
  /** The questions Hide found that still wait on the operator, oldest first (`open_asks`, the list 내 차례 reads). */
  asks?: readonly AskLine[];
  /** 무시하기 and 되돌리기 on one of `asks`, as 내 차례 does it for the session; a view without it (an ended session's record) draws no question line. */
  onAsk?: (id: string, status: "open" | "dismissed") => void;
  /** Drawn in the input box's place, as a sleeping session's wake line is (B43). */
  foot?: ReactNode;
  /** The session's terminal, which a record it cannot read points to; none for a session whose pane is gone. */
  onTerminal?: () => void;
  /** The conversation was scrolled to its top: the page before it can be read. */
  onReachTop?: () => void;
} & ThreadPartActions) {
  const { t } = useInterfaceTranslation();
  const scroller = useThreadScroll(lines, onReachTop);
  const turns = useMemo(() => threadTurns(lines), [lines]);
  const running = job?.state === "working" || job?.state === "starting";
  // The session's own folder and machine, where its relative paths are read.
  const place = useShellStore(useShallow((s) => (job?.pane_id ? answerPlace(s.rest, job.pane_id) : null)));
  const words = useWords(mentions, actions.onSession, actions.onCopy, place, actions.onFile);
  // A press again on the same question lights it again for the whole time.
  const [lit, setLit] = useState<{ id: string; press: number } | null>(null);
  useEffect(() => {
    if (lit === null) return undefined;
    const timer = window.setTimeout(() => setLit(null), LIT_MS);
    return () => window.clearTimeout(timer);
  }, [lit]);
  // A question goes to the answer it was asked in; one asked before every page read so far goes to the top, which reads the page before.
  const goTo = (ask: AskLine) => {
    const box = scroller.ref.current;
    if (!box) return;
    const id = askedIn(turns, ask.at_unix_ms)?.id ?? null;
    const message = id === null ? null : box.querySelector<HTMLElement>(`[data-desk-thread-message="${CSS.escape(id)}"]`);
    if (id === null || !message) {
      box.scrollTo({ top: 0 });
      return;
    }
    message.scrollIntoView({ block: "center" });
    setLit((now) => ({ id, press: (now?.press ?? 0) + 1 }));
  };
  // 본문 보기: the block of the answer a question was asked in that holds the sentence asking it, as the conversation draws it; the whole answer for a question kept without its sentence, or one the answer read here does not hold.
  // `askedIn` gives the last answer line written by then, while the core checked the sentence against the turn's assistant messages together, so a sentence in an earlier message of that turn is not found here and the whole of the last one shows.
  const askedWords = (ask: AskLine) => {
    const answer = askedIn(turns, ask.at_unix_ms);
    if (!answer) return null;
    const text = (ask.quote ? markdownBlockHolding(answer.text, ask.quote) : null) ?? answer.text;
    return <MarkdownText text={text} chips={words.chips} copyCode={words.copy} />;
  };
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-desk-thread={job?.id ?? ""}>
      <div ref={scroller.ref} onScroll={scroller.onScroll} data-desk-thread-conversation="true" className="flex min-h-0 min-w-0 flex-1 flex-col gap-md overflow-y-auto px-md py-md">
        {failure ? (
          <p role="alert" className="flex min-w-0 flex-wrap items-center gap-x-sm gap-y-xxs text-caption" data-desk-thread-failure={failure}>
            <CircleAlertIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-warning" />
            <span className="min-w-0 text-subtle-foreground">{t(failure === "no_record" ? "desk.thread.readFailed.no_record" : "desk.thread.readFailed.refused")}</span>
            {onTerminal ? <Button variant="text-link" size="inline" onClick={onTerminal}>{t("desk.thread.toTerminal")}</Button> : null}
          </p>
        ) : reading ? (
          <p role="status" className="animate-pulse text-caption text-muted-foreground" data-desk-thread-reading="true">{t("desk.thread.reading")}</p>
        ) : lines.length === 0 ? (
          <p className="text-caption text-muted-foreground" data-desk-thread-empty="true">{t("desk.thread.empty")}</p>
        ) : (
          <ol className="flex min-w-0 flex-col gap-lg" data-desk-thread-lines={lines.length}>
            {turns.map((turn, index) => <Turn key={turn.key} turn={turn} live={running && index === turns.length - 1} handed={startedByDesk && index === 0} words={words} lit={lit?.id ?? null} actions={actions} />)}
          </ol>
        )}
        {/* The working line Desk's conversation draws too (feedback 1 c3), until the turn ends or waits on the operator. */}
        {running && !reading && !failure ? <WorkingLine phase={threadPhase(turns.at(-1))} since={workingSince} /> : null}
        {ask}
      </div>
      {onAsk ? <ThreadAsks asks={asks} onGo={goTo} onSet={onAsk} body={askedWords} /> : null}
      {foot ?? (
        <div className="shrink-0 px-sm pb-sm">
          <DeskComposer
            value={composer.value}
            label={t("desk.thread.composerLabel", { title: job?.title ?? "" })}
            placeholder={t("desk.thread.placeholder")}
            tray={composer.tray}
            focused={composer.focused}
            held={composer.held}
            notice={composer.notice}
            onChange={composer.onChange}
            onSend={composer.onSend}
          />
        </div>
      )}
    </div>
  );
}
