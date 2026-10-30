import { ArrowRightIcon, GitBranchIcon } from "lucide-react";
import type { Actions } from "./actions";
import { diffNoticeText } from "./diffNotice";
import { PlainPatchView } from "./editor/PatchView";
import { useInterfaceTranslation } from "./i18n/client";
import type { ProjectSuggestions, SuggestionBranch, SuggestionRow } from "./snapshot";
import { useShellStore } from "./store";
import { attemptLine } from "./suggestionList";
import { BranchButtons } from "./SuggestionsTab";
import { cn } from "./lib/utils";

// The 보기 slot's branch card (PRD hide-ai-branch-first-apply D-02, D-03,
// B4): a Hide AI branch waiting for the operator, its name and base, each
// file it changes with its +/−, and the row's three choices in its head. A
// file opens its diff against the base in the same slot, read on the
// Project's machine; the checkout in front never moves for it. The card
// reads its row from the 제안 tab the rail keeps open for the Project in
// front, so a row that left it, or another Project's, says so instead.

/** The row a branch item names, while the 제안 tab holds it ready. */
function useBranchRow(workspaceId: string, rowId: string): { project: ProjectSuggestions; row: SuggestionRow; branch: SuggestionBranch } | null {
  const project = useShellStore((state) => state.suggestions?.project ?? null);
  if (!project || project.workspace_id !== workspaceId) return null;
  const row = project.rows.find((candidate) => candidate.id === rowId);
  const branch = row?.attempt?.branch;
  if (!row || !branch || (row.attempt?.phase !== "ready" && row.attempt?.phase !== "merging")) return null;
  return { project, row, branch };
}

export function BranchCardView({ workspaceId, rowId, path, actions }: { workspaceId: string; rowId: string; path: string | null; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const target = useBranchRow(workspaceId, rowId);
  if (!target) return <p className="p-lg text-caption text-muted-foreground" data-branch-card-missing={rowId}>{t("suggestions.card.unavailable")}</p>;
  const { project, row, branch } = target;
  const line = attemptLine(t, row);
  const base = branch.base ?? "main";
  return (
    <article aria-label={t("suggestions.card.aria", { name: branch.name })} className="flex min-h-0 min-w-0 flex-1 flex-col [overflow-wrap:anywhere] text-foreground" data-branch-card={row.id} data-branch-card-path={path ?? undefined}>
      <div className="flex shrink-0 flex-col gap-sm border-b border-border p-lg">
        <div className="flex min-w-0 items-center gap-xs text-caption">
          <GitBranchIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
          <span className="min-w-0 truncate font-mono text-foreground" title={branch.name}>{branch.name}</span>
          <ArrowRightIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
          <span className="shrink-0 font-mono text-muted-foreground">{base}</span>
        </div>
        {line?.branch?.again ? <span className="text-caption text-warning" data-branch-card-failure={row.attempt?.failure ?? undefined}>{line.text}</span> : null}
        {row.attempt?.phase === "merging" ? <span className="text-caption text-muted-foreground">{t("suggestions.attempt.merging")}</span> : <BranchButtons row={row} project={project} actions={actions} again={line?.branch?.again ?? null} className="col-start-auto pt-0" />}
      </div>
      {path ? <BranchFileDiff workspaceId={workspaceId} rowId={rowId} path={path} /> : <Files branch={branch} onOpen={(file) => actions.openBranchCard(workspaceId, rowId, file)} />}
    </article>
  );
}

/** Each changed file with its +/−; a press opens its diff (B4). */
function Files({ branch, onOpen }: { branch: SuggestionBranch; onOpen: (path: string) => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <section className="flex min-h-0 flex-col overflow-y-auto p-lg" aria-label={t("suggestions.card.files")} data-branch-card-files={branch.files_total}>
      <div className="flex items-center gap-xs pb-xs text-caption">
        <span className="font-medium text-foreground">{t("suggestions.card.files")}</span>
        <span className="text-muted-foreground">{branch.files_total}</span>
        <span className="ml-auto font-mono text-micro text-pr-mergeable">+{branch.added}</span>
        <span className="font-mono text-micro text-pr-failed">−{branch.removed}</span>
      </div>
      <ul className="flex flex-col" role="list">
        {branch.files.map((file) => (
          <li key={file.path}>
            <button type="button" onClick={() => onOpen(file.path)} className="flex w-full min-w-0 items-center gap-sm rounded-xs px-xs py-xs text-left text-caption outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring" data-branch-card-file={file.path} title={file.path}>
              <span className="min-w-0 flex-1 truncate font-mono">{file.path}</span>
              {file.binary ? <span className="shrink-0 text-micro text-muted-foreground">{t("suggestions.card.binary")}</span> : (
                <>
                  <span className="shrink-0 font-mono text-micro text-pr-mergeable">+{file.added}</span>
                  <span className="shrink-0 font-mono text-micro text-pr-failed">−{file.removed}</span>
                </>
              )}
            </button>
          </li>
        ))}
        {branch.files_total > branch.files.length ? <li className="py-xs text-micro text-muted-foreground">{t("suggestions.card.filesMore", { shown: branch.files.length, total: branch.files_total })}</li> : null}
      </ul>
    </section>
  );
}

/** One file's diff against the base, as the core read it for this card or for the open 제안 row; `inline` draws it inside the row, without the path head. */
export function BranchFileDiff({ workspaceId, rowId, path, inline = false }: { workspaceId: string; rowId: string; path: string; inline?: boolean }) {
  const { t } = useInterfaceTranslation();
  const diff = useShellStore((state) => state.rest?.pr_work?.branch_diffs?.find((shown) => shown.workspace_id === workspaceId && shown.row === rowId && shown.path === path) ?? null);
  const scaleValue = useShellStore((state) => state.rest?.ui_state?.editor_text_scale);
  const scale = typeof scaleValue === "number" ? scaleValue : 1;
  return (
    <section className={cn("flex min-h-0 min-w-0 flex-col", inline ? "max-h-(--size-relationship-list-max) overflow-auto rounded-xs border border-border" : "flex-1")} aria-label={path} data-branch-card-diff={path}>
      {inline ? null : <div className="shrink-0 truncate border-b border-border px-lg py-xs font-mono text-caption text-muted-foreground" title={path}>{path}</div>}
      {!diff || diff.reading ? (
        <p className={cn("text-caption text-muted-foreground", inline ? "p-sm" : "p-lg")} data-branch-card-diff-state="loading">{t("suggestions.card.readingDiff")}</p>
      ) : diff.failed ? (
        <p className={cn("text-caption text-warning", inline ? "p-sm" : "p-lg")} data-branch-card-diff-state="failed">{t("suggestions.card.diffFailed")}</p>
      ) : (
        <>
          {diff.notice ? <div className="border-b border-border px-md py-xs text-caption text-warning" data-diff-notice={diff.notice.code}>{diffNoticeText(diff.notice, t)}</div> : null}
          {diff.text ? <PlainPatchView text={diff.text} wrap={inline} scale={scale} /> : <div className="min-h-0 flex-1" data-diff-empty="true" />}
        </>
      )}
    </section>
  );
}
