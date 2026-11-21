import type { CSSProperties, HTMLAttributes, ReactNode } from "react";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import type { SidebarTreeLine } from "../sidebarTree";
import type { AgentRow, SubagentRow } from "../snapshot";
import { TreeChevron, TreeRails } from "./agent-tree";
import { Elapsed } from "./elapsed";

/**
 * A subagent's mark (docs/UI_BEHAVIOR.md, Subagents): a small working-blue dot,
 * the one the work map circles a session with, smaller than a status mark and
 * never read as a child's circle or a resting agent's dashed ring. It stands
 * in the status mark's slot so a subagent line lines up with a child's.
 */
export function SubagentMark({ className, ...rest }: HTMLAttributes<HTMLSpanElement>) {
  return (
    <span aria-hidden="true" data-subagent-mark="" className={cn("inline-flex size-(--size-agent-mark) shrink-0 items-center justify-center", className)} {...rest}>
      <span className="size-(--size-subagent-dot) rounded-full bg-agent-working opacity-(--opacity-secondary)" />
    </span>
  );
}

/** The subagents a row lists, oldest first, and how many it counts beyond them. */
export function subagentsOf(agent: Pick<AgentRow, "subagents" | "subagent_rows">): { rows: SubagentRow[]; more: number } {
  const rows = agent.subagent_rows ?? [];
  return { rows, more: Math.max(0, (agent.subagents ?? 0) - rows.length) };
}

/** A subagent's words for its accessible name and hint: its kind and task, or the plain word when the agent says neither. */
export function subagentWords(t: ReturnType<typeof useInterfaceTranslation>["t"], row: SubagentRow): string {
  return [row.kind, row.task].filter(Boolean).join(" · ") || t("agents.subagents.unnamed");
}

/**
 * The text sizes of a subagent's line: the desktop lists' caption, or the
 * phone list's body. A line has nothing to press, so the phone's takes no
 * touch target's height and stays lighter than a child's row.
 */
const LINE_SIZE = {
  caption: { line: "min-h-(--size-sidebar-line) gap-xs text-caption", time: "text-micro" },
  body: { line: "gap-sm py-xs text-body", time: "text-body" },
} as const;
type LineSize = keyof typeof LINE_SIZE;

/**
 * One subagent as a line under its agent: its mark, its kind, what it was
 * asked to do and how long it has run, dimmed and with nothing to press,
 * since a subagent has no pane to open. A kind or task the agent does not say
 * leaves only the plain word and the time. The list it stands in gives it its
 * role: a list item under a row, a disabled option in the child popover.
 * `size` is the list's: caption on the desktop, body on the phone.
 */
export function SubagentLine({ row, size = "caption", className, ...rest }: { row: SubagentRow; size?: LineSize } & HTMLAttributes<HTMLDivElement>) {
  const { t } = useInterfaceTranslation();
  const named = row.kind ?? (row.task ? null : t("agents.subagents.unnamed"));
  return (
    <div title={subagentWords(t, row)} data-subagent-row={row.id} className={cn("flex min-w-0 items-center opacity-(--opacity-dimmed)", LINE_SIZE[size].line, className)} {...rest}>
      <SubagentMark />
      {named ? <span className="max-w-1/2 shrink-0 truncate text-subtle-foreground">{named}</span> : null}
      {row.task ? <span className="min-w-0 flex-1 truncate text-muted-foreground">{row.task}</span> : <span className="flex-1" />}
      <Elapsed since={row.started_unix_ms} className={cn("shrink-0 font-mono text-muted-foreground", LINE_SIZE[size].time)} />
    </div>
  );
}

/**
 * The line that says how many subagents run beyond the `listed` ones: `외 N개`,
 * or with none listed, as for an agent that names no subagent by id,
 * `서브에이전트 N개 작업 중`.
 */
export function SubagentMoreLine({ count, listed, size = "caption", className }: { count: number; listed: number; size?: LineSize; className?: string }) {
  const { t } = useInterfaceTranslation();
  return (
    <div data-subagent-more={count} className={cn("flex min-w-0 items-center text-muted-foreground opacity-(--opacity-dimmed)", LINE_SIZE[size].line, className)}>
      <SubagentMark />
      <span className="truncate">{listed > 0 ? t("agents.subagents.more", { count }) : t("agents.subagents.working", { count })}</span>
    </div>
  );
}

/**
 * A subagent's line in a sidebar tree, under its opened agent after its
 * children, on the same rails: a list item with nothing to press and no hover,
 * since there is no pane to open. `children` end the line as they end the
 * list's other rows, such as the project tree's fold slot, so its time stands
 * in their time column.
 */
export function SubagentTreeRow({ line, style, className, children }: { line: Extract<SidebarTreeLine, { kind: "subagent" | "subagents-more" }>; style?: CSSProperties; className?: string; children?: ReactNode }) {
  return (
    <li className={cn("flex min-w-0 items-stretch pr-xs", className)} style={style} data-tree-subagent={line.parent}>
      <TreeRails place={line.place} />
      <TreeChevron name="" open={null} hangs chain={line.place.chain} />
      {line.kind === "subagent" ? <SubagentLine row={line.subagent} className="min-w-0 flex-1 pl-xs" /> : <SubagentMoreLine count={line.count} listed={line.listed} className="min-w-0 flex-1 pl-xs" />}
      {/* The same gap a row's head keeps before what ends it. */}
      {children ? <span className="flex shrink-0 pl-xs">{children}</span> : null}
    </li>
  );
}
