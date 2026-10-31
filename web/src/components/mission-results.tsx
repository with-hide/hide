import type { ReactNode } from "react";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { missionPulls, missionResultCounts } from "../missions";
import { PR_LOOK } from "../prMark";
import type { Mission } from "../snapshot";
import { MissionPrMark } from "./mission-pr";
import { RowButton } from "./ui/row-button";

// What a finished or cancelled mission left, first under its head (PRD
// mission-tool D-07, B20): four result tiles counted from the record (merged
// pull requests, finish lines met with the skipped ones, the workers and
// checkers bound to it, its changes in main), a tile the record holds
// nothing for not drawn, then each node's pull request with its mark, title
// and state word; pressing one picks its node on the map.

export function ResultTiles({ mission }: { mission: Mission }) {
  const { t } = useInterfaceTranslation();
  const counts = missionResultCounts(mission);
  const tiles: { key: string; value: string; label: ReactNode }[] = [];
  if (counts.merged) tiles.push({ key: "merged", value: String(counts.merged.merged), label: t("mission.tile.merged") });
  if (counts.criteria) {
    const { passed, total, skipped } = counts.criteria;
    tiles.push({ key: "criteria", value: `${passed} / ${total}`, label: skipped > 0 ? `${t("mission.tile.criteria")} · ${t("mission.tile.skipped", { count: skipped })}` : t("mission.tile.criteria") });
  }
  if (counts.agents) {
    const { workers, checkers } = counts.agents;
    tiles.push({ key: "agents", value: String(workers), label: checkers > 0 ? t("mission.tile.checkers", { count: checkers }) : t("mission.tile.workers") });
  }
  if (counts.landed) tiles.push({ key: "landed", value: `${counts.landed.landed} / ${counts.landed.total}`, label: t("mission.tile.landed") });
  if (tiles.length === 0) return null;
  return (
    <dl aria-label={t("mission.results")} className="mx-md mb-sm grid grid-cols-2 gap-xs mission-wide:flex" data-mission-results={tiles.map((tile) => tile.key).join(" ")}>
      {tiles.map((tile) => (
        <div key={tile.key} className="flex min-w-0 flex-col-reverse gap-xxs rounded-md border border-border bg-card px-sm py-xs mission-wide:flex-1" data-mission-tile={tile.key}>
          <dt className="min-w-0 break-words text-caption text-muted-foreground">{tile.label}</dt>
          <dd className="text-headline font-semibold tabular-nums" data-mission-tile-value="true">{tile.value}</dd>
        </div>
      ))}
    </dl>
  );
}

export function PullList({ mission, onSelect }: { mission: Mission; onSelect: (key: string) => void }) {
  const { t } = useInterfaceTranslation();
  const pulls = missionPulls(mission);
  if (pulls.length === 0) return null;
  return (
    <ul aria-label={t("mission.prs")} className="mx-md mb-sm flex min-w-0 flex-col overflow-hidden rounded-md border border-border" data-mission-pulls={pulls.map(({ pr }) => pr.number).join(" ")}>
      {pulls.map(({ node, pr }) => (
        <li key={pr.number} className="min-w-0 border-b border-border last:border-b-0">
          <RowButton onClick={() => onSelect(node.key)} title={node.title} className="gap-sm rounded-none px-sm text-body" data-mission-pull={pr.number}>
            <MissionPrMark number={pr.number} state={pr.state} stale={mission.github_stale} />
            <span className="min-w-0 flex-1 truncate">{node.title}</span>
            {pr.state ? <span className={cn("shrink-0 text-caption", PR_LOOK[pr.state].tone)} data-mission-pull-state={pr.state}>{t(PR_LOOK[pr.state].word)}</span> : null}
          </RowButton>
        </li>
      ))}
    </ul>
  );
}
