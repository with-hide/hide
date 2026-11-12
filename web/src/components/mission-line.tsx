// A mission node card's bottom line as the desktop's map and the phone draw
// it (PRD mission-destination D-52-D-54, D-56, B55-B57, B59): the branch line
// of a node that changes code and the result line of one that does not, from
// `branchLine` and `resultLine` (`web/src/missionLine.ts`). The pull
// request's icon and colour are the PR list's (`PR_LOOK`), and so are those of
// the pull request a stacked node goes into, drawn before its number (D-57). A line with a
// number is one row while it fits, the number a step further from the branch
// than the icon is from the number; otherwise the number keeps the first row
// and `branch → destination` takes a second one from the card's left edge. A
// line without a number stays one row. Where the branch and the destination
// do not both fit, they share the row equally, each keeping its whole name
// when it needs less than half, and a cut one ends in `…` with its whole name
// on hover. A numbered line with marks at its right end (`⚠`, 내 차례, `!`)
// keeps the number and the marks on the first row, the marks taking only the
// width left, and puts `branch → destination` on the second. The card's
// accessible name says these words; the line itself is for the eye.

import type { ReactNode } from "react";
import { ChevronRightIcon, FileTextIcon, GitBranchIcon, GitPullRequestDraftIcon, GitPullRequestIcon, HourglassIcon, RocketIcon, SearchIcon, ShieldCheckIcon, UserRoundIcon, type LucideIcon } from "lucide-react";
import { useInterfaceTranslation } from "../i18n/translator";
import { cn } from "../lib/utils";
import { linePrLabel, lineToLabel, lineToWords, resultLineWords, type BranchLine, type LineMark, type ResultLine, type ResultTone } from "../missionLine";
import { PR_LOOK } from "../prMark";
import type { MissionNodePreset, PrState } from "../snapshot";

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

/** `end` is the foot's right-end marks, drawn on the line's first row; `whole` draws every name in full, wrapping, as the card's hover does (D-58). */
export function BranchLineView({ line, end, whole = false, className }: { line: BranchLine; end?: ReactNode; whole?: boolean; className?: string }) {
  const { t } = useInterfaceTranslation();
  const { icon: Icon, tone } = markLook(line.mark);
  const due = line.person === "due";
  // Only another branch is bold (B55); the pull request a stacked node goes into is its mark and a grey number, so the node's own number stays the line's lead (D-57).
  const bold = line.to?.kind === "branch" && line.to.elsewhere;
  const plan = line.plan ? t(line.plan === "pr" ? "mission.foot.planPr" : "mission.foot.planDirect") : null;
  // A direct change's `PR 없이` follows the destination's name, or the branch when there is none, and never gives way.
  const direct = (after: boolean) => (line.direct ? <span className="shrink-0 font-normal text-muted-foreground" data-line-direct="true">{after ? "· " : ""}{t("mission.foot.planDirect")}</span> : null);
  const from = plan != null || line.branch != null || (line.direct && line.to == null);
  const icon = <Icon className={cn("size-(--size-icon-sm) shrink-0", tone)} data-line-icon={line.mark} />;
  const to = (full: boolean) =>
    line.to ? (
      <span className={cn("flex min-w-0 items-center gap-xxs", full && "flex-wrap", due ? "font-semibold text-mission-hold" : bold ? "font-bold text-foreground" : "text-subtle-foreground")} data-line-to={line.to.kind} data-line-elsewhere={bold || undefined}>
        <span className={cn("shrink-0", !due && "text-muted-foreground")}>→</span>
        {line.person ? <UserRoundIcon className={cn("size-(--size-icon-sm) shrink-0", PERSON_TONE[line.person])} data-line-person-mark={line.person} /> : null}
        {line.to.kind === "stack" ? <StackMark state={line.to.state} /> : null}
        <span className={cn(full ? "break-all" : "min-w-0 truncate", line.to.kind === "stack" && "font-normal tabular-nums text-muted-foreground")} title={full ? undefined : lineToWords(line.to, t)}>{lineToLabel(line.to)}</span>
        {direct(true)}
      </span>
    ) : null;
  // A line runs inline when its head (the number, or the icon of a line without one that has marks) holds the first row
  // and the path wraps whole to the second when it does not fit beside it.
  const inline = line.pr ? !end : end != null;
  const path = (
    // Where the branch and the destination do not both fit, a grid of auto columns grows them from zero alike, each frozen
    // at its whole width, so a name shorter than half the row stays whole and two long ones share it.
    <span aria-hidden="true" className={cn("min-w-0 max-w-full auto-cols-[minmax(0,auto)] grid-flow-col items-center justify-start gap-x-xxs", inline ? "inline-grid h-lh align-top" : "grid", line.pr && end && "col-span-2 justify-self-start", !line.pr && !end && "flex-1")} data-line-path="true">
      {from ? (
        <span className="flex min-w-0 items-center gap-xxs" data-line-from="true">
          {plan ? <span className="min-w-0 truncate" data-line-plan={line.plan}>{plan}</span> : null}
          {line.branch ? (
            <span className="min-w-0 truncate" title={line.branch.name} data-line-branch={line.branch.name}>
              {line.branch.outside ? "↗ " : ""}
              {line.branch.name}
            </span>
          ) : null}
          {line.to ? null : direct(line.branch != null)}
        </span>
      ) : null}
      {to(false)}
    </span>
  );
  const data = { "data-node-line": "branch", "data-line-mark": line.mark, "data-line-person": line.person ?? undefined };
  const number = line.pr ? <span className="text-foreground tabular-nums" data-line-pr={line.pr.number}>{linePrLabel(line.pr)}</span> : null;
  if (whole) {
    return (
      <span className={cn("flex min-w-0 flex-wrap items-center gap-x-xxs text-caption text-muted-foreground", className)} {...data}>
        <span className="flex items-center gap-xxs" data-line-head="true">
          {icon}
          {number}
        </span>
        {plan ? <span data-line-plan={line.plan}>{plan}</span> : null}
        {line.branch ? (
          <span className="break-all" data-line-branch={line.branch.name}>
            {line.branch.outside ? "↗ " : ""}
            {line.branch.name}
          </span>
        ) : null}
        {line.to ? to(true) : direct(line.branch != null)}
      </span>
    );
  }
  // A numbered line with marks: the number and the marks on the first row, the number never giving way, and the path on
  // the second. A numbered line without marks runs inline, so the path wraps whole under the number to a row of the card's
  // whole width. A line without a number that has marks runs inline the same way, its icon in the number's place and the
  // marks at the first row's right end, never wider than the icon leaves so a warning's words give way first: the path
  // stays on that row when it fits between them and otherwise takes the whole second row. Without marks it is one row that
  // never wraps.
  if (line.pr && end) {
    return (
      <span className={cn("grid min-w-0 grid-cols-[auto_minmax(0,1fr)] items-center gap-x-xs text-caption text-muted-foreground", className)} {...data}>
        <span aria-hidden="true" className="flex h-lh items-center gap-xxs" data-line-head="true">
          {icon}
          {number}
        </span>
        {end}
        {path}
      </span>
    );
  }
  return inline ? (
    <span className={cn("flow-root min-w-0 text-caption text-muted-foreground", className)} {...data}>
      <span aria-hidden="true" className="mr-xs inline-flex h-lh items-center gap-xxs align-top" data-line-head="true">
        {icon}
        {number}
      </span>
      {end ? <span className="float-right ml-xs flex h-lh max-w-[calc(100%_-_var(--size-icon-sm)_-_2_*_var(--spacing-xs))] items-center">{end}</span> : null}
      {path}
    </span>
  ) : (
    <span className={cn("flex min-w-0 items-center gap-x-xs text-caption text-muted-foreground", className)} {...data}>
      <span aria-hidden="true" className="flex h-lh shrink-0 items-center" data-line-head="true">{icon}</span>
      {path}
    </span>
  );
}

/** The pull request a stacked node goes into, as the PR list draws it by its state; grey when no node of the mission holds it (D-57). */
function StackMark({ state }: { state: PrState | null }) {
  const { icon: Icon, tone } = state ? PR_LOOK[state] : OWN_MARK.pull;
  return <Icon className={cn("size-(--size-icon-sm) shrink-0", tone)} data-line-to-mark={state ?? "unknown"} />;
}

/** A general node's preset as its icon (PRD node-presets D-08): research a magnifier, verify a shield with a check, deploy a rocket, none a document; the core's preset table names the same icons. */
export const PRESET_ICON: Record<MissionNodePreset | "general", LucideIcon> = { research: SearchIcon, verify: ShieldCheckIcon, deploy: RocketIcon, general: FileTextIcon };

/** The icon's colour by where the node stands (v4): grey planned, blue worked, green done, red on a failed verdict. */
const RESULT_TONE: Record<ResultTone, string> = { planned: "text-muted-foreground", working: "text-mission-working", done: "text-mission-done", failed: "text-mission-failed" };

export function ResultLineView({ line, whole = false, className }: { line: ResultLine; whole?: boolean; className?: string }) {
  const { t } = useInterfaceTranslation();
  const Icon = PRESET_ICON[line.preset ?? "general"];
  const verdict = line.shows === "verdict" ? (line.verdict === "pass" ? "font-semibold text-mission-done" : "font-semibold text-mission-failed") : null;
  const words = resultLineWords(line, t);
  return (
    <span aria-hidden="true" className={cn("flex min-w-0 items-center gap-xxs text-caption", line.shows === "conclusion" ? "text-subtle-foreground" : "text-muted-foreground", className)} data-node-line="result" data-line-kind={line.preset ?? "general"} data-line-tone={line.tone} data-line-shows={line.shows} data-line-verdict={line.shows === "verdict" ? line.verdict : undefined}>
      <Icon className={cn("size-(--size-icon-sm) shrink-0", RESULT_TONE[line.tone])} data-line-icon={line.preset ?? "general"} />
      <span className={cn(whole ? "min-w-0 break-words" : "min-w-0 truncate", verdict)} title={!whole && line.shows === "conclusion" ? line.line : undefined}>{words}</span>
      {line.shows !== "kind" ? <ChevronRightIcon className="size-(--size-icon-sm) shrink-0 text-muted-foreground opacity-(--opacity-dimmed)" /> : null}
    </span>
  );
}

/** A waiting node's place for a worker on the desktop's card and the phone's (PRD node-presets D-13, B12): an hourglass and `기다림`, since Hide, not an agent, watches its pull request or branch. */
export function WaitingMark() {
  const { t } = useInterfaceTranslation();
  return (
    <span className="flex shrink-0 items-center gap-xxs" data-node-waiting="true">
      <HourglassIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
      {t("mission.kind.external")}
    </span>
  );
}
