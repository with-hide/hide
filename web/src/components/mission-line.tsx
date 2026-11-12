// A mission node card's bottom line as the desktop's map and the phone draw
// it (PRD mission-destination D-52-D-54, D-56, B55-B57, B59): the branch line
// of a node that changes code and the result line of one that does not, from
// `branchLine` and `resultLine` (`web/src/missionLine.ts`). The pull
// request's icon and colour are the PR list's (`PR_LOOK`). A branch line is
// one row while it fits; otherwise `→ destination` moves to a row of its own
// that starts under the number, the first row's branch truncating if it must,
// and a destination longer than its row ends in `…`. The card's accessible
// name says these words; the line itself is for the eye.

import { ChevronRightIcon, GitBranchIcon, GitPullRequestDraftIcon, GitPullRequestIcon, SearchIcon, ShieldCheckIcon, UserRoundIcon, type LucideIcon } from "lucide-react";
import { useInterfaceTranslation } from "../i18n/translator";
import { cn } from "../lib/utils";
import { linePrLabel, lineToLabel, resultLineWords, type BranchLine, type LineMark, type ResultLine } from "../missionLine";
import { PR_LOOK } from "../prMark";

/** A mark the PR list does not draw: a branch before any pull request, one not read yet, and a planned node's faint marks. */
const OWN_MARK: Record<Exclude<LineMark, keyof typeof PR_LOOK>, { icon: LucideIcon; tone: string }> = {
  pull: { icon: GitPullRequestIcon, tone: "text-muted-foreground" },
  branch: { icon: GitBranchIcon, tone: "text-muted-foreground" },
  plan_pr: { icon: GitPullRequestDraftIcon, tone: "text-muted-foreground opacity-(--opacity-dimmed)" },
  plan_direct: { icon: GitBranchIcon, tone: "text-muted-foreground opacity-(--opacity-dimmed)" },
};

function markLook(mark: LineMark): { icon: LucideIcon; tone: string } {
  return mark in PR_LOOK ? PR_LOOK[mark as keyof typeof PR_LOOK] : OWN_MARK[mark as keyof typeof OWN_MARK];
}

/** The person before the destination (D-53): faint orange ahead, yellow when the merge is the operator's now, grey after. */
const PERSON_TONE = { ahead: "text-mission-hold/70", due: "text-mission-hold", merged: "text-muted-foreground" } as const;

export function BranchLineView({ line, className }: { line: BranchLine; className?: string }) {
  const { t } = useInterfaceTranslation();
  const { icon: Icon, tone } = markLook(line.mark);
  const due = line.person === "due";
  const bold = line.to != null && (line.to.kind === "stack" || line.to.elsewhere);
  return (
    <span aria-hidden="true" className={cn("flex min-w-0 items-start gap-xxs text-caption text-muted-foreground", className)} data-node-line="branch" data-line-mark={line.mark} data-line-person={line.person ?? undefined}>
      {/* The mark stands beside the first row, so a destination moved to the next row starts under the number. */}
      <span className="flex h-lh shrink-0 items-center">
        <Icon className={cn("size-(--size-icon-sm)", tone)} data-line-icon={line.mark} />
      </span>
      <span className="flex min-w-0 flex-1 flex-wrap items-center gap-x-xxs" data-line-path="true">
        {/* Where the work is: the plan's words, or the number and the branch; only the branch gives way. */}
        <span className="flex min-w-0 items-center gap-xxs" data-line-from="true">
          {line.plan ? <span className="shrink-0" data-line-plan={line.plan}>{t(line.plan === "pr" ? "mission.foot.planPr" : "mission.foot.planDirect")}</span> : null}
          {line.pr ? <span className="shrink-0 text-foreground tabular-nums" data-line-pr={line.pr.number}>{linePrLabel(line.pr)}</span> : null}
          {line.branch ? (
            <span className="min-w-0 truncate" title={line.branch.name} data-line-branch={line.branch.name}>
              {line.branch.outside ? "↗ " : ""}
              {line.branch.name}
            </span>
          ) : null}
        </span>
        {line.to ? (
          // Where it goes wraps to the next row whole rather than cutting into it; only one longer than a row is cut, at its end.
          <span className={cn("flex max-w-full shrink-0 items-center gap-xxs", due ? "font-semibold text-mission-hold" : bold ? "font-bold text-foreground" : "text-subtle-foreground")} data-line-to={line.to.kind} data-line-elsewhere={bold || undefined}>
            <span className={cn("shrink-0", !due && "text-muted-foreground")}>→</span>
            {line.person ? <UserRoundIcon className={cn("size-(--size-icon-sm) shrink-0", PERSON_TONE[line.person])} data-line-person-mark={line.person} /> : null}
            <span className="min-w-0 truncate" title={lineToLabel(line.to)}>{lineToLabel(line.to)}</span>
          </span>
        ) : null}
        {line.direct ? <span className="shrink-0" data-line-direct="true">{line.branch || line.to ? "· " : ""}{t("mission.foot.planDirect")}</span> : null}
      </span>
    </span>
  );
}

const RESULT_ICON = { research: SearchIcon, verify: ShieldCheckIcon } as const;

export function ResultLineView({ line, className }: { line: ResultLine; className?: string }) {
  const { t } = useInterfaceTranslation();
  const Icon = RESULT_ICON[line.kind];
  const verdict = line.shows === "verdict" ? (line.verdict === "pass" ? "text-mission-done" : "text-mission-failed") : null;
  const words = resultLineWords(line, t);
  return (
    <span aria-hidden="true" className={cn("flex min-w-0 items-center gap-xxs text-caption", verdict ?? (line.shows === "conclusion" ? "text-subtle-foreground" : "text-muted-foreground"), className)} data-node-line="result" data-line-kind={line.kind} data-line-shows={line.shows} data-line-verdict={line.shows === "verdict" ? line.verdict : undefined}>
      <Icon className={cn("size-(--size-icon-sm) shrink-0", line.working && line.shows === "kind" ? "text-mission-working" : verdict)} />
      <span className="min-w-0 truncate" title={line.shows === "conclusion" ? line.line : undefined}>{words}</span>
      {line.shows !== "kind" ? <ChevronRightIcon className="size-(--size-icon-sm) shrink-0" /> : null}
    </span>
  );
}
