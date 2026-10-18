import { CheckIcon } from "lucide-react";
import type { HTMLAttributes } from "react";
import { markName } from "../agentRow";
import { statusText } from "../agentStatus";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import type { TaskState } from "../sessionWork";
import type { RowMark } from "../snapshot";

/**
 * The state a task is drawn in: work in progress only while its agent's row
 * draws a ring; a task the label left in progress on a stopped row is work
 * left, as its row says (status-marks D-17, B9, B34).
 */
export function shownTaskState(state: TaskState, mark: RowMark | null): TaskState {
  return state === "working" && mark !== "going" && mark !== "waiting" ? "unfinished" : state;
}

/**
 * A task's mark, a checkbox (PRD record-card-readability D-05, B7, B8): ☑
 * once done, ☐ otherwise. A live session's question or block puts the
 * operator's `!` in front of the box, a done task's too when its last END is
 * a question (`question`, PRD task-continuation D-07); an ended session has
 * no pane to answer, so its task says it in words instead. The agents' rings
 * and dots are the agent rows' and never a task's.
 */
export function TaskMark({ state, live, question = false }: { state: TaskState; live: boolean; question?: boolean }) {
  const { t } = useInterfaceTranslation();
  const done = state === "done";
  const asks = live && (state === "question" || state === "blocked" || (done && question));
  return <span role="img" aria-label={t(done ? "sessionWork.done" : "sessionWork.left")} data-task-state={state} className="flex h-(--size-sidebar-line) shrink-0 items-center gap-xxs">
    {asks ? <span aria-hidden="true" className="font-semibold text-warning" data-task-asks="true">!</span> : null}
    {done
      ? <span aria-hidden="true" className="flex size-(--size-icon-sm) items-center justify-center rounded-xs bg-success text-background" data-task-box="checked"><CheckIcon className="size-(--size-status-mark)" strokeWidth={3} /></span>
      : <span aria-hidden="true" className="size-(--size-icon-sm) rounded-xs border border-muted-foreground" data-task-box="empty" />}
  </span>;
}

/**
 * The words at a task's right end where its box does not say enough (B8,
 * B9): a live session's `내 차례 · 질문`, an ended one's `일 남음 · 질문하고
 * 끝남`, `일 남음` while work is left, and a pull request's state on a task
 * whose request's PR mark hangs on its first task (`prShown` false). Work in
 * progress, a done task and an unjudged one say nothing.
 */
export function TaskWord({ state, live, prShown }: { state: TaskState; live: boolean; prShown: boolean }) {
  const { t } = useInterfaceTranslation();
  const asked = state === "question" || state === "blocked";
  const word = asked
    ? live ? `${markName(t, "turn")} · ${statusText(t, state === "blocked" ? "error" : "question")}` : `${statusText(t, "unfinished")} · ${t(state === "blocked" ? "sessionWork.endedBlocked" : "sessionWork.endedAsking")}`
    : state === "unfinished" ? statusText(t, "unfinished")
      : (state === "review" || state === "ci_failed") && !prShown ? t(`sessionWork.${state}`) : null;
  if (!word) return null;
  return <span className={cn("shrink-0 text-micro leading-(--size-sidebar-line)", asked && live ? "text-warning" : "text-muted-foreground")} data-task-word={state}>{word}</span>;
}

/** A task's name: the box's neighbour, dimmed once done. */
export function TaskName({ state, name, className, ...rest }: { state: TaskState; name: string } & HTMLAttributes<HTMLSpanElement>) {
  return <span className={cn("min-w-0 flex-1 whitespace-normal leading-(--size-sidebar-line) [overflow-wrap:anywhere]", state === "done" ? "text-subtle-foreground" : "text-foreground", className)} {...rest}>{name}</span>;
}
