// A code node's station line on its card (PRD mission-destination D-13-D-16,
// D-21, B24-B30): a disc and a fixed word per station, joined left to right,
// the last one the branch the result lands in. Yellow is only what the
// operator does now, faint orange a human step planned ahead, grey what has
// passed or what the Conductor handles, lime where the result lands. The line
// is drawn from `nodeStations` (`web/src/missionStations.ts`), which the
// desktop and the phone share; the card's accessible name says its words.

import { ArrowDownToLineIcon, CheckIcon, EyeIcon, GitBranchIcon, GitPullRequestClosedIcon, GitPullRequestIcon, HammerIcon, ThumbsUpIcon, UserRoundIcon, XIcon, type LucideIcon } from "lucide-react";
import { Fragment } from "react";
import { useInterfaceTranslation } from "../i18n/translator";
import { cn } from "../lib/utils";
import { stationLabel, type Station, type StationState } from "../missionStations";

const ICON: Record<Station["kind"], LucideIcon> = { work: HammerIcon, branch: GitBranchIcon, pr: GitPullRequestIcon, approve: ThumbsUpIcon, gate: EyeIcon, land: ArrowDownToLineIcon };

/** A station's mark: a check once passed or landed, a cross on failed checks, a closed pull request's mark, `👤` on a last station the operator merges. */
function iconOf(station: Station): LucideIcon {
  if (station.state === "done" || station.state === "landed") return CheckIcon;
  if (station.state === "failed") return XIcon;
  if (station.state === "closed") return GitPullRequestClosedIcon;
  if (station.kind === "land" && station.person) return UserRoundIcon;
  return ICON[station.kind];
}

const DISC: Record<StationState, string> = {
  todo: "border-border text-muted-foreground",
  current: "border-mission-working text-mission-working ring-2 ring-mission-working/20",
  done: "border-border bg-border text-foreground",
  failed: "border-mission-failed text-mission-failed",
  closed: "border-border text-muted-foreground",
  ahead: "border-dashed border-mission-hold bg-mission-hold/10 text-mission-hold",
  due: "border-2 border-mission-hold bg-mission-hold/20 text-mission-hold",
  landed: "border-mission-land bg-mission-land text-background",
};

const WORD: Record<StationState, string> = {
  todo: "text-muted-foreground",
  current: "text-mission-working",
  done: "text-muted-foreground",
  failed: "text-mission-failed",
  closed: "text-muted-foreground",
  ahead: "text-mission-hold",
  due: "font-semibold text-mission-hold",
  landed: "text-mission-land",
};

/** The last station waiting on no one: lime, the place the result goes. */
const LAND_TODO = "border-mission-land text-mission-land";

export function StationLine({ stations, className }: { stations: readonly Station[]; className?: string }) {
  const { t } = useInterfaceTranslation();
  return (
    <span aria-hidden="true" className={cn("flex min-w-0 items-start", className)} data-node-stations={stations.map((station) => `${station.kind}:${station.state}`).join(" ")}>
      {stations.map((station, index) => {
        const last = index === stations.length - 1;
        const Icon = iconOf(station);
        const label = stationLabel(station, t);
        const land = station.kind === "land";
        // A result going somewhere other than the mission's destination ends in bold, so it reads without a hover (D-14).
        const elsewhere = land && station.place.kind !== "destination";
        const quiet = land && !station.person && (station.state === "todo" || station.state === "current");
        // A branch's name may be long: the last station's, and an outside branch's first, truncate within half the line, so the stations between keep their words (B30).
        const named = last || station.kind === "branch";
        const outside = (station.kind === "pr" && station.outside) || station.kind === "branch";
        return (
          <Fragment key={station.kind}>
            {index > 0 ? <span className="mt-[calc(var(--size-mission-station)/2)] h-(--size-mission-edge) min-w-xs flex-1 bg-border" /> : null}
            <span className={cn("flex flex-col items-center gap-xxs", named ? "min-w-0 max-w-1/2 shrink" : "shrink-0")} data-station={station.kind} data-station-state={station.state} data-station-place={land ? station.place.kind : undefined}>
              <span className={cn("flex size-(--size-mission-station) shrink-0 items-center justify-center rounded-full border", quiet ? cn(LAND_TODO, station.state === "current" && "ring-2 ring-mission-land/20") : DISC[station.state])}>
                <Icon className="size-(--size-agent-mark)" strokeWidth={2.25} />
              </span>
              <span className={cn("text-micro whitespace-nowrap", named && "max-w-full truncate", quiet ? "text-mission-land" : WORD[station.state], elsewhere && "font-bold text-foreground")} title={named ? label : undefined} data-station-label="true" data-node-outside={outside || undefined}>
                {label}
              </span>
            </span>
          </Fragment>
        );
      })}
    </span>
  );
}
