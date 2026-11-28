// A conversation's tools (mockup v3 sections 1, 2, 4): one gray line per run with its counts by kind, the run's lines
// opened under it, and the call still running under the owl. Every mark is a gray icon of its kind; color is only a
// failure's red and the changed lines' green and red. Plain values in, no screen state: the phone draws them too.

import { ChevronDownIcon, ChevronRightIcon, CircleHelpIcon, FilePenIcon, FileSearchIcon, GlobeIcon, SquareTerminalIcon, WrenchIcon, type LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { callName, type AcpContentBlock } from "../../desk";
import { useInterfaceTranslation } from "../../i18n/translator";
import { cn } from "../../lib/utils";
import { useRunningClock } from "../elapsed";
import { Button } from "../ui/button";
import { diffCounts, diffLines, type DiffLine } from "./diff";
import { toolMark, type RunCounts, type ToolLine, type ToolMark, type ToolRun } from "./rows";

const MARK_ICON: Record<ToolMark, LucideIcon> = {
  command: SquareTerminalIcon,
  edit: FilePenIcon,
  read: FileSearchIcon,
  web: GlobeIcon,
  question: CircleHelpIcon,
  other: WrenchIcon,
};

/** A tool's kind as its gray icon: 명령, 파일 고침, 읽기·찾기, 웹, 질문, and every other call. */
export function ToolMarkIcon({ mark, className }: { mark: ToolMark; className?: string }) {
  const Icon = MARK_ICON[mark];
  return <Icon aria-hidden="true" className={cn("size-(--size-icon-sm) shrink-0 text-muted-foreground", className)} data-tool-mark={mark} />;
}

/** Lines added and taken away, `+n` in green and `−n` in red; a side with none is left out unless `both`. */
export function ChangeCounts({ added, removed, both = false }: { added: number; removed: number; both?: boolean }) {
  return (
    <span className="shrink-0 font-mono text-micro" data-tool-change={`${added}:${removed}`}>
      {added || both ? <span className="text-success">+{added}</span> : null}
      {(added || both) && (removed || both) ? " " : null}
      {removed || both ? <span className="text-destructive">−{removed}</span> : null}
    </span>
  );
}

/** A run's counts in words, in the order the gray line says them: commands, failures, reads, searches, web, changes, other tools. */
function RunWords({ counts }: { counts: RunCounts }) {
  const { t } = useInterfaceTranslation();
  const parts: ReactNode[] = [];
  if (counts.thoughts) parts.push(<span key="thoughts">{t("conversation.run.thoughts")}</span>);
  if (counts.commands) parts.push(<span key="commands">{t("conversation.run.commands", { count: counts.commands })}</span>);
  if (counts.failed) parts.push(<span key="failed" className="text-destructive" data-tool-failed={counts.failed}>{t("conversation.run.failed", { count: counts.failed })}</span>);
  if (counts.read) parts.push(<span key="read">{t("conversation.run.read", { count: counts.read })}</span>);
  if (counts.searched) parts.push(<span key="searched">{t("conversation.run.searched", { count: counts.searched })}</span>);
  if (counts.fetched) parts.push(<span key="fetched">{t("conversation.run.fetched", { count: counts.fetched })}</span>);
  if (counts.edited) {
    parts.push(
      <span key="edited" className="inline-flex items-center gap-xxs">
        {t("conversation.run.edited", { count: counts.edited })}
        {counts.added || counts.removed ? <ChangeCounts added={counts.added} removed={counts.removed} /> : null}
      </span>,
    );
  }
  if (counts.other) parts.push(<span key="other">{t("conversation.run.other", { count: counts.other })}</span>);
  if (parts.length === 0) parts.push(<span key="work">{t("conversation.run.work")}</span>);
  return (
    <span className="min-w-0 truncate">
      {parts.flatMap((part, at) => (at === 0 ? [part] : [<span key={`dot${at}`} aria-hidden="true"> · </span>, part]))}
    </span>
  );
}

/**
 * A run of tools between two pieces of the agent's words, as one gray line (mockup v3 section 2): its mark, its counts
 * by kind (`명령 10개 실행 · 1개 실패 · 파일 2개 읽음 · 파일 1개 고침 +182 −0`) and a chevron, right while folded and down
 * while its lines stand open under it. Pressing it opens or folds them in place. Without `onToggle` (the phone, whose
 * runs never open) it is the same mark and words alone, with no button and no chevron.
 */
export function ToolRunLine({ run, open, onToggle }: { run: ToolRun; open: boolean; onToggle?: () => void }) {
  if (!onToggle) {
    return (
      <p className="flex max-w-full min-w-0 items-center gap-xs self-start text-body text-muted-foreground" data-conversation-run="still">
        <ToolMarkIcon mark={run.mark} />
        <RunWords counts={run.counts} />
      </p>
    );
  }
  const Chevron = open ? ChevronDownIcon : ChevronRightIcon;
  return (
    <Button variant="text" size="inline" className="max-w-full min-w-0 self-start gap-xs text-body text-muted-foreground hover:text-foreground" aria-expanded={open} data-conversation-run={open ? "open" : "folded"} onClick={onToggle}>
      <ToolMarkIcon mark={run.mark} />
      <RunWords counts={run.counts} />
      <Chevron aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
    </Button>
  );
}

/**
 * A call's head (B25): its kind's mark, its one-line name (`callName`: a command as the command itself, a search with
 * its term; its first line when `firstLine`), `+n −m` after an edit, `실행 중` with how long it has run while a command
 * runs, and `실패` after it failed.
 */
export function ToolHead({ line, firstLine = false }: { line: ToolLine; firstLine?: boolean }) {
  const { t } = useInterfaceTranslation();
  const { added, removed } = diffCounts(line.diffs);
  const title = callName(line);
  const shown = firstLine ? (title.trim().split("\n", 1)[0] ?? "") : title;
  const running = line.status === "in_progress" || line.status === "pending";
  const clock = useRunningClock(running && line.toolKind === "execute" ? line.startedAtMs : null);
  return (
    <p className="flex min-w-0 items-center gap-xs text-caption text-muted-foreground">
      <ToolMarkIcon mark={toolMark(line.toolKind)} />
      <span className="min-w-0 truncate font-mono text-subtle-foreground" title={title}>{shown}</span>
      {line.diffs.length ? <ChangeCounts added={added} removed={removed} both /> : null}
      {running ? <span className="shrink-0 text-micro" data-desk-tool-clock={clock ?? undefined}>{clock ? `${t("desk.thread.tool.running")} ${clock}` : t("desk.thread.tool.running")}</span> : null}
      {line.status === "failed" ? <span className="shrink-0 text-micro text-destructive">{t("desk.thread.tool.failed")}</span> : null}
    </p>
  );
}

/** An image the session saw or made, from its bytes; a cut image keeps only its place. */
export function ThreadImage({ block }: { block: AcpContentBlock }) {
  const { t } = useInterfaceTranslation();
  if (block.type !== "image" || !block.data) return null;
  return <img src={`data:${block.mimeType};base64,${block.data}`} alt={t("desk.thread.image")} className="max-h-(--size-overview-graph-root-height) max-w-full rounded-sm border border-border object-contain" data-desk-thread-image="true" />;
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

/** A call of an opened run (B25): its head, then its diff and what came out, cut at four lines. */
export function ToolLineView({ line }: { line: ToolLine }) {
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

/**
 * The call the session runs now, one live line right under the owl (mockup v3 section 1): its head's first line,
 * never what came out, set in from the owl so it reads as what the owl does.
 */
export function LiveToolLine({ line }: { line: ToolLine }) {
  return (
    <div className="min-w-0 pl-(--size-working-owl) ml-xs" data-conversation-live={line.id}>
      <ToolHead line={line} firstLine />
    </div>
  );
}
