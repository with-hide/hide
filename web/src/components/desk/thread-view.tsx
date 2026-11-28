import { CheckIcon, ChevronDownIcon, ChevronRightIcon, ChevronUpIcon, CircleAlertIcon, FilePenIcon, FileSearchIcon, GlobeIcon, MailIcon, PlusIcon, SquareTerminalIcon, WrenchIcon } from "lucide-react";
import { pullLabel } from "../../codeHost";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ComponentPropsWithRef, type ReactNode, type Ref, type RefObject } from "react";
import { Trans } from "react-i18next";
import type { AcpContentBlock, AcpPlanEntry, AcpToolKind, DeskJob, DeskMention, DeskOutput, DeskSessionRef, ThreadHidePart, ThreadLine, ThreadReadFailure, ThreadTurn } from "../../desk";
import { askPlace, callName, namedMention, threadPhase, threadTurns } from "../../desk";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import { MarkdownText, type MarkdownChip, type MarkdownMark } from "../../MarkdownText";
import type { CopyText } from "../human-turn-copy";
import type { AskLine } from "../open-asks";
import { useRunningClock } from "../elapsed";
import { PlanChecklist } from "../plan-checklist";
import { PrMark } from "../pr-mark";
import { StatusMark } from "../status-mark";
import { Button } from "../ui/button";
import { useOutsidePressSurface } from "../ui/layer";
import { Hint } from "../ui/tooltip";
import { DeskComposer, SentAttachments } from "./desk-composer";
import type { ComposerMenus } from "./composer-menus";
import { sentWords, type ComposerTray } from "../../deskComposer";
import { DeskMissionChip } from "../mission-chip";
import { WorkingLine } from "./working-line";
import { SessionChip, useSessionChipBridge } from "./session-chip";
import { DeskOutputs } from "./desk-outputs";
import type { AnswerPlace, FoundAnswerPath } from "../../answerPaths";
import { useFilePathChips } from "./file-path-chip";
import { ThreadAsks } from "./thread-asks";
import { keepScroll, keptScroll, placeAt, scrollPlace, stillAt, type ScrollPlace } from "../../conversationScroll";
import { rowEstimate, threadRows, type RowGap, type ThreadRow, type ViewLine } from "../../threadRows";
import { defaultRangeExtractor, useVirtualizer, type Range, type VirtualItem, type Virtualizer } from "@tanstack/react-virtual";

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

/** A tool call's head (B25): its kind's icon, its one-line name (`callName`: a command as the command itself, a search with its term; its first line when `firstLine`), `+n −m` after an edit, `실행 중` in words while it runs (a ring is a session's state, D-75) and `실패` after it failed. */
function ToolHead({ line, firstLine = false }: { line: Extract<ThreadLine, { kind: "tool" }>; firstLine?: boolean }) {
  const { t } = useInterfaceTranslation();
  const Icon = TOOL_ICON[line.toolKind] ?? WrenchIcon;
  const { added, removed } = diffCounts(line.diffs);
  const title = callName(line);
  const shown = firstLine ? (title.trim().split("\n", 1)[0] ?? "") : title;
  const running = line.status === "in_progress" || line.status === "pending";
  const clock = useRunningClock(running && line.toolKind === "execute" ? line.startedAtMs : null);
  return (
    <p className="flex min-w-0 items-center gap-xs text-caption">
      <Icon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
      <span className="min-w-0 truncate font-mono text-subtle-foreground" title={title}>{shown}</span>
      {line.diffs.length ? <span className="shrink-0 font-mono text-micro"><span className="text-success">+{added}</span> <span className="text-destructive">−{removed}</span></span> : null}
      <span className="flex-1" />
      {running ? <span className="shrink-0 text-micro text-muted-foreground" data-desk-tool-clock={clock ?? undefined}>{clock ? `${t("desk.thread.tool.running")} ${clock}` : t("desk.thread.tool.running")}</span> : null}
      {line.status === "failed" ? <span className="shrink-0 text-micro text-destructive">{t("desk.thread.tool.failed")}</span> : null}
    </p>
  );
}

/** The lines a call's diffs add and take away, as its head counts them. */
function diffCounts(diffs: Extract<ThreadLine, { kind: "tool" }>["diffs"]): { added: number; removed: number } {
  const lines = diffs.flatMap((diff) => diffLines(diff.oldText, diff.newText));
  return { added: lines.filter((one) => one.sign === "+").length, removed: lines.filter((one) => one.sign === "-").length };
}

/** A `hide` call as the newest step of a running turn: the command the session typed, its first line, as a shell call's head reads. */
function HideCallHead({ command }: { command: string }) {
  return (
    <p className="flex min-w-0 items-center gap-xs text-caption">
      <SquareTerminalIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
      <span className="min-w-0 truncate font-mono text-subtle-foreground" title={command}>{command.trim().split("\n", 1)[0]}</span>
    </p>
  );
}

/** A tool call (B25): its head, then its diff and what came out, cut at four lines. */
function ToolLine({ line }: { line: Extract<ThreadLine, { kind: "tool" }> }) {
  const { t } = useInterfaceTranslation();
  const diffs = line.diffs.map((diff) => ({ path: diff.path, lines: diffLines(diff.oldText, diff.newText) }));
  return (
    <div className="flex min-w-0 flex-col gap-xxs" data-desk-thread-line="tool" data-tool-status={line.status}>
      <ToolHead line={line} />
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
  outputOf: (output: DeskOutput, viewId: string | null) => DeskOutput;
  onOutput: (output: DeskOutput) => void;
  /** A session a chip or an `@name` names: its pane when Hide knows it, else its name alone. */
  onSession: (session: DeskSessionRef) => void;
  /** Puts a message's or a code block's words on the clipboard (Doop E2). */
  onCopy: CopyText;
  /** A file path the session's words name, pressed (feedback 2 frame M [2]); without it paths stay words. */
  onFile?: (file: FoundAnswerPath) => void;
  /** Where the session's words were written, its machine and its own folder, where its relative paths are read: its pane's while the pane is there, its record's once it is gone; without it paths stay words. */
  place?: AnswerPlace | null;
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
      body = <div data-desk-thread-line="hide" data-desk-hide-part="view"><DeskOutputs outputs={[actions.outputOf(part.output, part.view_id)]} onOpen={actions.onOutput} /></div>;
      break;
    case "mission":
      body = (
        <PartLine kind="mission" icon={null}>
          <DeskMissionChip mission={part.mission} onOpen={actions.onMission} />
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
      return <AgentWords line={line} words={words} lit={null} live={false} />;
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
 * words always under it and on every code block. `lit` is the sentence of it a
 * question Hide found asks, lit for a moment after the question line brought
 * it into view (Doop N3 가).
 */
function AgentWords({ line, words, lit, live }: { line: Extract<ThreadLine, { kind: "agent" }>; words: Words; lit: MarkdownMark | null; live: boolean }) {
  // The file paths it names become chips once the turn has said them (feedback 2 frame M [2]).
  const files = useFilePathChips(line.text, words.place, live, words.file);
  const chips = useMemo(() => (files.length ? [...words.chips, ...files] : words.chips), [words.chips, files]);
  return (
    <div className="-mx-xs min-w-0 rounded-md px-xs" data-desk-thread-line="agent" data-desk-thread-message={line.id} data-desk-thread-lit={lit ? "true" : undefined}>
      <MarkdownText text={line.text} size="chat" chips={chips} copyCode={words.copy} mark={lit} />
    </div>
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
function UserLine({ line, handed, actions }: { line: Extract<ThreadLine, { kind: "user" }>; handed: boolean; actions: ThreadPartActions }) {
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
        <p data-desk-thread-bubble="true" className="max-w-(--size-pr-popover) whitespace-pre-wrap break-keep rounded-xl bg-secondary px-md py-xs text-subhead leading-relaxed text-foreground [overflow-wrap:anywhere]">{sent.words}</p>
      ) : null}
      {line.images.map((block, index) => <ThreadImage key={index} block={block} />)}
    </div>
  );
}

/**
 * A turn's fold line (B25): `작업 · 파일 n · 명령 m ›` above a finished turn's answer, `작업 중 · 명령 n · 파일 n ⌄` at
 * the running turn's foot, whose lines open above it, so its chevron points up while they stand. Whether it is open is
 * the conversation's, by the turn's key, so it outlives the line leaving the viewport.
 */
function TurnToggle({ turn, live, open, onToggle }: { turn: ThreadTurn; live: boolean; open: boolean; onToggle: (key: string) => void }) {
  const { t } = useInterfaceTranslation();
  const files = turn.files ? t("desk.turn.files", { count: turn.files }) : null;
  const commands = turn.commands ? t("desk.turn.commands", { count: turn.commands }) : null;
  // While it runs the line leads with what it does and counts the commands first; a finished turn's reads `작업 · 파일 n · 명령 m`.
  const counts = (live ? [commands, files] : [files, commands]).filter(Boolean).map((count) => ` · ${count}`).join("");
  const Chevron = live ? (open ? ChevronUpIcon : ChevronDownIcon) : open ? ChevronDownIcon : ChevronRightIcon;
  return (
    <Button variant="text" size="inline" className="self-start gap-xxs text-caption text-muted-foreground" aria-expanded={open} data-desk-turn-toggle={open ? "open" : "folded"} data-desk-turn-live={live || undefined} onClick={() => onToggle(turn.key)}>
      <span>{live ? <span className="text-foreground">{t("desk.turn.working")}</span> : t("desk.turn.work")}{counts}</span>
      <Chevron aria-hidden="true" className="size-(--size-icon-sm)" />
    </Button>
  );
}

/** A line of a turn's work, or the views it opened one after another as one row of tiles (feedback 2, frame M 나). */
function WorkRow({ row, words, actions }: { row: ThreadLine | ViewLine[]; words: Words; actions: ThreadPartActions }) {
  return Array.isArray(row) ? (
    <div data-desk-thread-line="hide" data-desk-hide-part="view"><DeskOutputs outputs={row.map((line) => actions.outputOf(line.part.output, line.part.view_id))} onOpen={actions.onOutput} /></div>
  ) : (
    <WorkLine line={row} words={words} actions={actions} />
  );
}

/** The room above a row, the space the conversation keeps between turns, a turn's parts and a group's lines. */
const GAP: Record<RowGap, string> = { none: "", turn: "pt-lg", part: "pt-sm", member: "pt-xs", foot: "pt-md" };

/**
 * What one row draws (B25, B26, D-75): the operator's words, the fold line, a line of the work (along the unfolded
 * work's rail), an answer message, a folded turn's part or its outputs, the running turn's newest call under its fold
 * line (its head's first line, never what came out), or the foot.
 */
function RowBody({ row, words, lit, actions, onToggle, foot }: { row: ThreadRow; words: Words; lit: Lit | null; actions: ThreadPartActions; onToggle: (key: string) => void; foot: ReactNode }) {
  switch (row.kind) {
    case "user":
      return <UserLine line={row.line} handed={row.handed} actions={actions} />;
    case "toggle":
      return <TurnToggle turn={row.of} live={row.live} open={row.open} onToggle={onToggle} />;
    case "work":
      return row.rail ? (
        <div className={cn("ml-xxs min-w-0 border-l border-border pl-sm", row.gap === "member" && GAP.member)} data-desk-thread-work="true">
          <WorkRow row={row.row} words={words} actions={actions} />
        </div>
      ) : <WorkRow row={row.row} words={words} actions={actions} />;
    case "answer":
      return <AgentWords line={row.line} words={words} lit={lit?.id === row.line.id ? lit.sentence : null} live={row.live} />;
    case "part":
      return <HidePartLine part={row.line.part} command={row.line.command} actions={actions} />;
    case "outputs":
      return <DeskOutputs outputs={row.lines.flatMap((line) => (line.part.kind === "view" ? [actions.outputOf(line.part.output, line.part.view_id)] : []))} onOpen={actions.onOutput} />;
    case "newest":
      return (
        <div className="ml-xxs min-w-0 border-l border-border pl-sm" data-desk-thread-newest={row.call.id}>
          {row.call.kind === "tool" ? <ToolHead line={row.call} firstLine /> : <HideCallHead command={row.call.command ?? ""} />}
        </div>
      );
    case "foot":
      return foot;
  }
}

/** How near an end of the conversation still counts as at it, in pixels. */
const AT_END = 24;
/** The rows drawn past each edge of the viewport, so a scroll shows drawn rows before the next are measured. */
const OVERSCAN = 6;
/** The room above the first row and below the last, `--spacing-md`. */
const LIST_PAD = 12;

/** A run of rows by their keys, both ends included; `null` reaches the conversation's first or last row. */
type RowSpan = readonly [string | null, string | null];

/**
 * The rows a selection in the conversation covers, by key, while one stands: a drag across messages, or one that
 * scrolled past the viewport, keeps every row between its ends drawn, so the browser copies all of it. An end outside
 * the conversation reaches its first or last row.
 */
function useSelectedRows(box: RefObject<HTMLDivElement | null>): RowSpan | null {
  const [span, setSpan] = useState<RowSpan | null>(null);
  useEffect(() => {
    const element = box.current;
    if (!element) return undefined;
    const document = element.ownerDocument;
    const rowAt = (node: Node, edge: null): string | null => {
      const at = node instanceof Element ? node : node.parentElement;
      const row = at?.closest<HTMLElement>("[data-thread-row]");
      return row && element.contains(row) ? row.dataset.threadRow! : edge;
    };
    const changed = () => {
      const selection = document.getSelection();
      let next: RowSpan | null = null;
      if (selection && !selection.isCollapsed && selection.rangeCount > 0) {
        const range = selection.getRangeAt(0);
        if (range.intersectsNode(element)) next = [rowAt(range.startContainer, null), rowAt(range.endContainer, null)];
      }
      setSpan((now) => (now?.[0] === next?.[0] && now?.[1] === next?.[1] && (now === null) === (next === null) ? now : next));
    };
    document.addEventListener("selectionchange", changed);
    return () => document.removeEventListener("selectionchange", changed);
  }, [box]);
  return span;
}

/** What a press on a message's words still gives to the message: a chip, a box, a code block's `복사`. */
const PRESSABLE = "a, button, input, textarea, select, label, [role='button'], [contenteditable='true']";

/**
 * Leaves a press on a message's words to the browser, so a drag selects across messages as on any page (issue
 * 3101): a message is a read-only editor, which would otherwise take the press and keep the selection inside itself.
 * The press stops above the editor, so the surface is marked for the open layers, as the terminal's is
 * (`useOutsidePressSurface`); a press on something a message draws to be pressed, and a ⌘- or Ctrl-press that opens a
 * link, still reaches it. A copy of a
 * selection that runs past one message is the browser's too, which copies every message it covers; the browser
 * hands a copy to the message the selection starts in, which would copy only its own part. A selection inside one
 * message is still copied by that message, as its Markdown.
 */
function useBrowserSelection(box: RefObject<HTMLDivElement | null>) {
  const surface = useOutsidePressSurface();
  useEffect(() => {
    const element = box.current;
    if (!element) return undefined;
    surface(element);
    const press = (event: MouseEvent) => {
      const target = event.target instanceof Element ? event.target : null;
      // A ⌘- or Ctrl-press is a message's own: it opens the link under it.
      if (event.button !== 0 || event.metaKey || event.ctrlKey || !target?.closest(".cm-content") || target.closest(PRESSABLE)) return;
      event.stopPropagation();
    };
    const messageOf = (node: Node) => (node instanceof Element ? node : node.parentElement)?.closest(".cm-content") ?? null;
    const copy = (event: ClipboardEvent) => {
      const selection = element.ownerDocument.getSelection();
      if (!selection || selection.isCollapsed || selection.rangeCount === 0) return;
      const range = selection.getRangeAt(0);
      const message = messageOf(range.startContainer);
      if (!message || message !== messageOf(range.endContainer)) event.stopPropagation();
    };
    element.addEventListener("mousedown", press, { capture: true });
    element.addEventListener("copy", copy, { capture: true });
    return () => {
      element.removeEventListener("mousedown", press, { capture: true });
      element.removeEventListener("copy", copy, { capture: true });
      surface(null);
    };
  }, [box, surface]);
}

/** Whether a row's change of height moves the scroll by as much (`useThreadList`). */
function keepsReadingPlace(item: VirtualItem, _delta: number, instance: Virtualizer<HTMLDivElement, Element>): boolean {
  const top = (instance.scrollOffset ?? 0) + instance.scrollAdjustments;
  return instance.itemSizeCache.has(item.key) ? item.end <= top : item.start < top;
}

/**
 * The conversation's rows near its scroll viewport, and its scrolling (B25, issue 3101). Only the rows within a few of
 * the viewport are drawn, each measured as it is drawn and kept by its key while it is not; the rows of `keep` are
 * drawn wherever they stand (a selection, the lit question, the place being restored) and so is the foot.
 *
 * While the operator is at its end, what the session writes next is brought into view; a page read in above, or a row
 * above the viewport changing height, leaves the rows they are reading where they were (`anchorTo: "end"`); reaching
 * the top asks for the page before. Left above its end for another screen or another session, `scrollKey`'s
 * conversation comes back to the row the operator was reading (`keptScroll`), held there while the rows around it are
 * measured until the operator scrolls; a row the lines no longer hold leaves it at its end.
 */
function useThreadList(ref: RefObject<HTMLDivElement | null>, rows: readonly ThreadRow[], scrollKey: string | null, onReachTop: (() => void) | undefined, keep: readonly RowSpan[]) {
  const list = useRef<HTMLOListElement>(null);
  const [kept] = useState(() => (scrollKey === null ? null : keptScroll(scrollKey)));
  const anchor = useRef<ScrollPlace | null>(kept);
  const atEnd = useRef(kept === null);
  const indexOf = useMemo(() => new Map(rows.map((row, index) => [row.key, index])), [rows]);
  const held = anchor.current?.anchor ?? null;
  const foot = rows.at(-1)?.kind === "foot" ? rows.length - 1 : null;
  // Every row a span covers, as indexes; recomputed when the rows or the spans move, not on every scroll.
  const drawn = useMemo(() => {
    const spans = held === null ? keep : [...keep, [held, held] as const];
    const indexes: number[] = foot === null ? [] : [foot];
    for (const [from, to] of spans) {
      const first = from === null ? 0 : indexOf.get(from);
      const last = to === null ? rows.length - 1 : indexOf.get(to);
      if (first === undefined || last === undefined) continue;
      for (let index = Math.min(first, last); index <= Math.max(first, last); index += 1) indexes.push(index);
    }
    return indexes;
  }, [keep, held, foot, indexOf, rows.length]);
  const rangeExtractor = useCallback((range: Range) => {
    if (drawn.length === 0) return defaultRangeExtractor(range);
    return [...new Set([...defaultRangeExtractor(range), ...drawn])].sort((a, b) => a - b);
  }, [drawn]);
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => ref.current,
    estimateSize: useCallback((index: number) => rowEstimate(rows[index]), [rows]),
    getItemKey: useCallback((index: number) => rows[index]?.key ?? index, [rows]),
    rangeExtractor,
    overscan: OVERSCAN,
    paddingStart: LIST_PAD,
    paddingEnd: LIST_PAD,
    anchorTo: "end",
    scrollEndThreshold: AT_END,
  });
  // A row above the viewport that changes height (an image that arrives late, words that wrap again) moves the
  // scroll by as much, whichever way the reader scrolls, so what they read stays put: a row measured for the first
  // time when its top is above the viewport, a row measured again when the whole of it is. The library's own rule
  // skips the second while scrolling up, which let the reading line move.
  virtualizer.shouldAdjustScrollPositionOnItemSizeChange = keepsReadingPlace;
  // What the rows no longer hold (a page let go, a thread read anew) is forgotten, so the measured heights stay as
  // many as the rows.
  useEffect(() => {
    for (const key of virtualizer.itemSizeCache.keys()) if (!indexOf.has(key as string)) virtualizer.itemSizeCache.delete(key);
  }, [indexOf, virtualizer]);
  useLayoutEffect(() => {
    const box = ref.current;
    return () => {
      if (box && scrollKey !== null) keepScroll(scrollKey, atEnd.current ? null : (anchor.current ?? scrollPlace(box)));
    };
  }, [scrollKey]);
  // To the foot by its offset, never by a row: a row's place moves as rows are measured, and a scroll aimed at a row
  // is aimed again for seconds, which would take back a reader who scrolled up meanwhile.
  const follow = useCallback(() => {
    const box = ref.current;
    if (box) virtualizer.scrollToOffset(box.scrollHeight);
  }, [ref, virtualizer]);
  const listed = rows.length > 0;
  useLayoutEffect(() => {
    const box = ref.current;
    const element = list.current;
    if (!box || !element) return undefined;
    // The held row is drawn at its estimated place, then placed again as the rows around it are measured; at the foot,
    // the rows growing as they are measured or as words stream in are followed.
    const observer = new ResizeObserver(() => {
      if (anchor.current) placeAt(box, anchor.current);
      else if (atEnd.current) follow();
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [listed, ref, follow]);
  useLayoutEffect(() => {
    const box = ref.current;
    if (!box || rows.length === 0) return;
    const place = anchor.current;
    if (place && !indexOf.has(place.anchor)) {
      anchor.current = null;
      atEnd.current = true;
    } else if (place) {
      placeAt(box, place);
    }
    if (!anchor.current && atEnd.current) follow();
  }, [rows]);
  const onScroll = () => {
    const box = ref.current;
    if (!box) return;
    if (anchor.current && stillAt(box, anchor.current)) return;
    anchor.current = null;
    atEnd.current = box.scrollHeight - box.scrollTop - box.clientHeight < AT_END;
    if (box.scrollTop < AT_END) onReachTop?.();
  };
  return { list, onScroll, virtualizer };
}

/** How long the sentence a question asks stays lit after the question line brought it into view (Doop N3 가). */
const LIT_MS = 2_000;
/** A lit message stands where it was brought once it has not moved for this many frames. */
const LIT_STILL_FRAMES = 3;
/** The most frames a lit message is brought to the middle again while the rows on its way are measured, about a second. */
const LIT_SEEK_FRAMES = 60;

const NO_OPEN: ReadonlySet<string> = new Set();
const NO_ROWS: readonly ThreadRow[] = [];

/** The message a pressed question went to, the sentence of it lit (none for a question kept without its sentence), and the question. */
type Lit = { id: string; sentence: MarkdownMark | null; ask: string };

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
export function ThreadView({ job, lines, reading = false, failure = null, mentions = NO_MENTIONS, startedByDesk = false, composer, ask = null, asks = NO_ASKS, onAsk, foot = null, workingSince = null, onTerminal, onReachTop, olderLeft = false, scrollKey = null, ...actions }: {
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
  /**
   * The input box: its words and files, the `/` and `@` menus Desk's composer has (`useComposerMenus`), and the bar
   * under it (`SessionAgentBar`); over it the pane's waiting line (`queue`, PRD desk-queue D-05), and with `onStop`
   * ■ stands in send's place while the agent works, ↵ still sending (B10).
   */
  composer: { value: string; tray: ComposerTray; focused?: boolean; held?: boolean; notice?: string | null; textRef?: Ref<HTMLTextAreaElement>; menus?: ComposerMenus; bar?: ReactNode; queue?: ReactNode; onStop?: () => void; onChange: (value: string, caret: number) => void; onCaret?: (caret: number) => void; onSend: () => void };
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
  /** The record holds pages before the ones read, which `onReachTop` reads. */
  olderLeft?: boolean;
  /** Which conversation this is, so the place it was left at comes back with it; none keeps no place. */
  scrollKey?: string | null;
} & ThreadPartActions) {
  const { t } = useInterfaceTranslation();
  const box = useRef<HTMLDivElement>(null);
  const selected = useSelectedRows(box);
  useBrowserSelection(box);
  const turns = useMemo(() => threadTurns(lines), [lines]);
  const running = job?.state === "working" || job?.state === "starting";
  const words = useWords(mentions, actions.onSession, actions.onCopy, actions.place ?? null, actions.onFile);
  // Which turns' work the operator unfolded, by key: the conversation's, so a fold outlives its rows leaving the viewport.
  const [open, setOpen] = useState<ReadonlySet<string>>(NO_OPEN);
  const toggle = useCallback((key: string) => setOpen((now) => {
    const next = new Set(now);
    if (!next.delete(key)) next.add(key);
    return next;
  }), []);
  // Where each question was asked, decided once for the line (`askPlace`): found, not known until the pages before are read, or gone.
  const settled = !reading && failure === null;
  const places = useMemo(() => new Map(asks.map((ask) => [ask.id, askPlace(turns, ask, { olderLeft, settled })])), [asks, turns, olderLeft, settled]);
  const gone = useMemo(() => new Set(asks.filter((ask) => places.get(ask.id)?.state === "gone").map((ask) => ask.id)), [asks, places]);
  // A pressed question not found yet reads the pages before, one at a time, until it is found or gone; a gone one does nothing.
  const [seek, setSeek] = useState<{ id: string; press: number } | null>(null);
  const [lit, setLit] = useState<(Lit & { press: number }) | null>(null);
  // The working line Desk's conversation draws too (feedback 1 c3), until the turn ends or waits on the operator, and the agent's own question.
  const working = running && !reading && !failure;
  const listed = settled && lines.length > 0;
  const rows = useMemo(() => (listed ? threadRows(turns, { running, open, handed: startedByDesk, foot: working || ask !== null }) : NO_ROWS), [listed, turns, running, open, startedByDesk, working, ask !== null]);
  // A selection's rows stay drawn while it stands, and so does the message a question lights while it is lit.
  const litId = lit?.id ?? null;
  const keep = useMemo<RowSpan[]>(() => [...(selected ? [selected] : []), ...(litId ? [[litId, litId] as const] : [])], [selected, litId]);
  const scroller = useThreadList(box, rows, scrollKey, onReachTop, keep);
  // A fold of a turn the conversation no longer holds (its page let go) is forgotten with it.
  useEffect(() => {
    if (!settled || open.size === 0) return;
    const held = new Set(turns.map((turn) => turn.key));
    if ([...open].some((key) => !held.has(key))) setOpen(new Set([...open].filter((key) => held.has(key))));
  }, [turns, open, settled]);
  const goTo = (ask: AskLine) => {
    if (places.get(ask.id)?.state === "gone") return;
    setSeek((now) => ({ id: ask.id, press: Math.max(now?.press ?? 0, lit?.press ?? 0) + 1 }));
  };
  useEffect(() => {
    if (!seek) return;
    const place = places.get(seek.id);
    if (!place || place.state === "gone") {
      setSeek(null);
      return;
    }
    if (place.state === "found") {
      setSeek(null);
      setLit({ id: place.line.id, sentence: place.sentence, ask: seek.id, press: seek.press });
      return;
    }
    if (olderLeft) {
      // At the top, the page before reads in above, where the operator is taken to look.
      if (box.current) box.current.scrollTop = 0;
      onReachTop?.();
    }
  }, [seek, places, olderLeft, onReachTop]);
  useEffect(() => {
    if (!lit) return undefined;
    // The message is drawn wherever it stands while it is lit, then brought to the middle; the rows on the way are
    // measured as they are drawn and move it, so it is brought there again until it stays, for at most a second. Its
    // sentence is drawn lit a frame later, inside the message's own editor, and is what is brought there once drawn.
    let frame = 0;
    let frames = 0;
    let still = 0;
    let last: number | null = null;
    const center = () => {
      const message = box.current?.querySelector<HTMLElement>(`[data-desk-thread-message="${CSS.escape(lit.id)}"]`);
      const target = message?.querySelector<HTMLElement>("[data-markdown-lit]") ?? message;
      if (target) {
        const top = target.getBoundingClientRect().top;
        if (top === last) still += 1;
        else {
          target.scrollIntoView({ block: "center" });
          last = target.getBoundingClientRect().top;
          still = 0;
        }
      }
      frames += 1;
      if (still < LIT_STILL_FRAMES && frames < LIT_SEEK_FRAMES) frame = window.requestAnimationFrame(center);
    };
    center();
    const timer = window.setTimeout(() => setLit(null), LIT_MS);
    return () => {
      window.cancelAnimationFrame(frame);
      window.clearTimeout(timer);
    };
  }, [lit]);
  const footNode = (
    <div className="flex min-w-0 flex-col gap-md">
      {working ? <WorkingLine phase={threadPhase(turns.at(-1))} since={workingSince} /> : null}
      {ask}
    </div>
  );
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-desk-thread={job?.id ?? ""}>
      <div ref={box} onScroll={scroller.onScroll} data-desk-thread-conversation="true" className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto px-md [overflow-anchor:none]">
        {listed ? (
          // Only the rows near the viewport are drawn, each where its measured height puts it (issue 3101).
          <ol ref={scroller.list} className="relative w-full min-w-0 shrink-0" style={{ height: scroller.virtualizer.getTotalSize() }} data-desk-thread-lines={lines.length}>
            {scroller.virtualizer.getVirtualItems().map((item) => {
              const row = rows[item.index]!;
              // A line inside the unfolded work keeps its room inside the rail, so the rail runs on unbroken.
              const inRail = row.kind === "work" && row.rail && row.gap === "member";
              return (
                <li key={item.key} ref={scroller.virtualizer.measureElement} data-index={item.index} data-thread-row={row.key} data-scroll-anchor={row.key} data-desk-thread-turn={row.turn || undefined} className={cn("absolute inset-x-0 flex min-w-0 flex-col", !inRail && GAP[row.gap])} style={{ top: item.start }}>
                  <RowBody row={row} words={words} lit={lit} actions={actions} onToggle={toggle} foot={footNode} />
                </li>
              );
            })}
          </ol>
        ) : (
          <div className="flex min-w-0 shrink-0 flex-col gap-md py-md">
            {failure ? (
              <p role="alert" className="flex min-w-0 flex-wrap items-center gap-x-sm gap-y-xxs text-caption" data-desk-thread-failure={failure}>
                <CircleAlertIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-warning" />
                <span className="min-w-0 text-subtle-foreground">{t(failure === "no_record" ? "desk.thread.readFailed.no_record" : "desk.thread.readFailed.refused")}</span>
                {onTerminal ? <Button variant="text-link" size="inline" onClick={onTerminal}>{t("desk.thread.toTerminal")}</Button> : null}
              </p>
            ) : reading ? (
              <p role="status" className="animate-pulse text-caption text-muted-foreground" data-desk-thread-reading="true">{t("desk.thread.reading")}</p>
            ) : (
              <p className="text-caption text-muted-foreground" data-desk-thread-empty="true">{t("desk.thread.empty")}</p>
            )}
            {working ? <WorkingLine phase={threadPhase(turns.at(-1))} since={workingSince} /> : null}
            {ask}
          </div>
        )}
      </div>
      {onAsk ? <ThreadAsks asks={asks} onGo={goTo} onSet={onAsk} lit={seek?.id ?? lit?.ask ?? null} gone={gone} /> : null}
      {foot ?? (
        <div className="flex shrink-0 flex-col gap-xs px-sm pb-sm">
          {composer.queue}
          <DeskComposer
            value={composer.value}
            label={t("desk.thread.composerLabel", { title: job?.title ?? "" })}
            placeholder={t("desk.thread.placeholder")}
            textRef={composer.textRef}
            tray={composer.tray}
            focused={composer.focused || composer.menus?.menuOpen}
            held={composer.held}
            notice={composer.notice}
            menu={composer.menus?.menu}
            menuOpen={composer.menus?.menuOpen}
            menuEmpty={composer.menus?.menuEmpty}
            onMenuOpenChange={composer.menus?.onMenuOpenChange}
            onKeyDown={composer.menus?.onKeyDown}
            onChange={composer.onChange}
            onCaret={composer.onCaret}
            onSend={composer.onSend}
            answering={composer.onStop !== undefined}
            stopHint={t("desk.queue.stopHint")}
            onStop={composer.onStop}
          >
            {composer.bar}
          </DeskComposer>
        </div>
      )}
    </div>
  );
}
