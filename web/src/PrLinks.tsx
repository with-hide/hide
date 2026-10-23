import { HouseIcon } from "lucide-react";
import { useRef, type ReactNode } from "react";
import { Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList, CommandSeparator } from "./components/ui/command";
import { Popover, PopoverAnchor, PopoverContent } from "./components/ui/popover";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import type { MessageKey } from "./i18n/catalogs";
import { folderName, sameIssue } from "./linkPanel";
import type { Checkout, LinkIssueSource, LinkPanel, Task, Workspace } from "./snapshot";
import { TaskGlyph } from "./issueMarks";
import { issueSourceName } from "./settings";
import { useUiStore } from "./ui";
import type { Actions } from "./actions";

// What a pull request is linked to (PRD link-graph B4, B5; PRD pr-card-and-
// session-record-redesign D-04, B2): its issues with where each link came
// from and the worktrees of its branch, in one quiet line under the card's
// meta line, and the Link issue picker the card and a list row's menu open.
// What is true now (the board's issue, the checkout) wins over the record.

const SOURCE_WORD: Record<LinkIssueSource, MessageKey> = { closes: "links.source.closes", hide: "links.source.hide" };

/** A GitHub task key's web address, for an issue the project's list no longer holds. */
function issueUrl(key: string): string | null {
  const match = /^github:([\w.-]+\/[\w.-]+)#(\d+)$/i.exec(key);
  return match ? `https://github.com/${match[1]}/issues/${match[2]}` : null;
}

function issueLabel(key: string): string {
  return `#${key.split("#").pop() ?? key}`;
}

/** Actual linked issues and branch worktrees; an absent link draws no placeholder. */
export function PrLinks({ issue: boardIssue, checkout: boardCheckout, record, project, actions }: { issue: { key: string } | null; checkout: Checkout | null; record: LinkPanel | null; project: Workspace; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const tasks = project.tasks?.tasks ?? [];
  const issues: { key: string; source: LinkIssueSource | null }[] = [...(record?.pr?.issues ?? [])];
  // The board's issue is GitHub's or the branch's link now; the record may not have it yet (B5).
  if (boardIssue && !issues.some((issue) => sameIssue(issue.key, boardIssue.key))) issues.push({ key: boardIssue.key, source: null });
  const paths = [...(record?.pr?.worktrees ?? [])];
  if (boardCheckout && !paths.includes(boardCheckout.path)) paths.unshift(boardCheckout.path);
  const checkouts = project.checkouts;
  if (!issues.length && !paths.length) return null;
  return (
    <p className="flex min-w-0 flex-wrap items-center gap-x-md text-caption text-muted-foreground" data-pr-panel-links="true">
      {issues.length > 0 ? <span className="flex min-w-0 flex-1 flex-col gap-y-xxs">
        {issues.map((issue) => {
          const task = tasks.find((candidate) => sameIssue(candidate.key, issue.key)) ?? null;
          return <IssueLine key={issue.key} issueKey={issue.key} source={issue.source} task={task} project={project} actions={actions} />;
        })}
      </span> : null}
      {paths.map((path) => {
        const here = checkouts.find((checkout) => checkout.path === path && checkout.exists) ?? null;
        return (
          <span key={path} className="inline-flex min-w-0 max-w-full items-center gap-xxs rounded-xs border border-border px-xs font-mono text-micro" data-pr-panel-worktree={path} data-removed={here ? undefined : "true"}>
            <HouseIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
            {here ? (
              <Hint label={path}>
                <button type="button" className="min-w-0 truncate rounded-xs text-subtle-foreground outline-none hover:underline focus-visible:ring-1 focus-visible:ring-ring" onClick={() => actions.openWorkspace(project.device_id, here.workspace_id, here.id)}>
                  {folderName(path)}
                </button>
              </Hint>
            ) : (
              <>
                <Hint label={path} reveals>
                  <span className="min-w-0 truncate">{folderName(path)}</span>
                </Hint>
                <span className="shrink-0 font-sans" data-pr-panel-cleaned="true">
                  {t("links.cleaned")}
                </span>
              </>
            )}
          </span>
        );
      })}
    </p>
  );
}

/** One linked issue (B4, B18): its item in the Overview's 백로그 when the project lists it, else GitHub; where the link came from is in the tooltip. */
function IssueLine({ issueKey, source, task, project, actions }: { issueKey: string; source: LinkIssueSource | null; task: Task | null; project: Workspace; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const label = task ? `${task.id ?? issueLabel(issueKey)} ${task.title}` : issueLabel(issueKey);
  const url = task?.url ?? issueUrl(issueKey);
  const open = task ? () => actions.openBacklogItem(project.device_id, project.id, task.key) : url ? () => actions.openPullRequest(url, project.device_id, true) : null;
  const hint = [task ? t("links.openIssue", { issue: label }) : t("prList.openIssue", { issue: label }), source ? t(SOURCE_WORD[source]) : null].filter(Boolean).join(" · ");
  const sentence = <><span className="text-muted-foreground">{t("links.issue")} </span>{label}</>;
  return (
    <span className="inline-flex min-w-0 items-center gap-xs" data-pr-panel-issue={issueKey} data-pr-panel-source={source ?? undefined}>
      {open ? (
        <Hint label={hint} reveals>
          <button type="button" className="min-w-0 truncate rounded-xs text-left text-subtle-foreground outline-none hover:underline focus-visible:ring-1 focus-visible:ring-ring" onClick={open}>
            {sentence}
          </button>
        </Hint>
      ) : (
        <span className="min-w-0 truncate text-subtle-foreground">{sentence}</span>
      )}
      {task ? <span className="shrink-0" data-pr-panel-issue-state={task.open ? "open" : "closed"}>· {t(task.open ? "links.issueOpen" : "links.issueClosed")}</span> : null}
    </span>
  );
}

/** The project's open issues and `Create new issue` (overview-lenses-prs B9) under Link issue, `children`; choosing one opens its confirmation, or for a Local issue links it. */
export function IssuePicker({ number, project, open, onOpenChange, children }: { number: number; project: Workspace; open: boolean; onOpenChange: (open: boolean) => void; children: ReactNode }) {
  const { t } = useInterfaceTranslation();
  const issues = (project.tasks?.tasks ?? []).filter((task) => task.open);
  // A choice hands the keyboard to the dialog it opens, not back to the button.
  const chosen = useRef(false);
  const choose = (dialog: Parameters<ReturnType<typeof useUiStore.getState>["setWorkspaceDialog"]>[0]) => {
    chosen.current = true;
    onOpenChange(false);
    useUiStore.getState().setWorkspaceDialog(dialog);
  };
  return (
    <Popover open={open} onOpenChange={onOpenChange}>
      <Hint label={t("prList.linkHint")} reveals>
        <PopoverAnchor asChild>{children}</PopoverAnchor>
      </Hint>
      <PopoverContent
        align="end"
        className="w-(--size-pr-popover) p-none"
        data-pr-link-picker={number}
        onOpenAutoFocus={() => {
          chosen.current = false;
        }}
        onCloseAutoFocus={(event) => {
          if (chosen.current) event.preventDefault();
        }}
      >
        <Command>
          <CommandInput placeholder={t("prList.searchIssues")} autoFocus />
          <CommandList>
            <CommandEmpty>{t("board.empty")}</CommandEmpty>
            <CommandGroup heading={project.tasks?.source ? issueSourceName(project.tasks.source, t) : t("issue.label")}>
              {issues.map((task) => (
                <CommandItem key={task.key} value={`${task.id ?? ""} ${task.title} ${task.key}`} onSelect={() => choose({ kind: "pr_link", workspaceId: project.id, prNumber: number, issueKey: task.key })} data-pr-link-choice={task.key}>
                  <TaskGlyph task={task} />
                  <span className="shrink-0 font-mono text-caption text-muted-foreground">{task.id}</span>
                  <span className="min-w-0 truncate">{task.title}</span>
                </CommandItem>
              ))}
            </CommandGroup>
            <CommandSeparator />
            <CommandGroup>
              <CommandItem value={t("prList.newIssue")} onSelect={() => choose({ kind: "pr_new_issue", workspaceId: project.id, prNumber: number })} data-pr-link-new="true">
                {t("prList.newIssue")}
              </CommandItem>
            </CommandGroup>
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}
