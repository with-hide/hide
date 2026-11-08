// The phone's missions tab (PRD mobile-companion-ui B28, B31, B32, B45, D-14):
// every mission this Mac holds in the Mission tool's own four groups, each
// row with its state, title, one line, a cell per step with `n/m`, the time
// it has run and, away from the Mac, its device. What waits on the operator,
// the groups and the line are `missionList.ts`'s rules; this draws them.

import { CheckIcon, ChevronRightIcon, Loader2Icon, XIcon } from "lucide-react";
import { PILL_TONE, ProgressCells, STATE_PILL_TONE } from "../components/mission-tone";
import { useElapsed } from "../components/elapsed";
import { RowButton } from "../components/ui/row-button";
import { Button } from "../components/ui/button";
import { useInterfaceTranslation } from "../i18n/translator";
import { formatDateTime } from "../i18n/format";
import { requireInterfaceLanguage } from "../i18n/locale";
import { CLOSED_SHOWN, groupMissions, missionPillKey, rowLine, rowLineText, type MissionEntry, type MissionGroup } from "../missionList";
import { missionCells } from "../missions";
import { cn } from "../lib/utils";
import { OfflineText } from "./OfflineText";
import { DeviceTag } from "./parts";
import { openCount, progress, useMissionEntries } from "./missionModel";
import type { PhoneMission } from "./protocol";
import { TabBar } from "./TabBar";
import { openMission, setMissionFold, usePhone } from "./store";

export function MissionsScreen() {
  const { t } = useInterfaceTranslation();
  const entries = useMissionEntries();
  const failed = usePhone((s) => s.missionsFailed);
  const omitted = usePhone((s) => s.missionsOmitted);
  const connected = usePhone((s) => s.connected);
  const unreachable = usePhone((s) => s.unreachable);
  // The last record stays, dimmed, under the unreachable line (B45).
  const dim = unreachable && !connected;
  return (
    <main className="flex min-h-full flex-col" data-phone-missions="true">
      <header className="phone-safe-top px-xl pb-sm">
        <h1 className="pt-lg text-headline font-semibold text-foreground">{t("mobile.missions.title")}</h1>
        {entries && entries.length > 0 ? <p className="mt-xs text-subhead text-muted-foreground" data-phone-missions-open={openCount(entries)}>{t("mobile.missions.running", { count: openCount(entries) })}</p> : null}
      </header>
      {dim ? (
        <div role="status" className="flex items-center gap-sm px-xl py-sm text-body text-subtle-foreground" data-phone-unreachable="true">
          <Loader2Icon aria-hidden="true" className="size-(--size-icon-lg) shrink-0 animate-spin" />
          <span><OfflineText /></span>
        </div>
      ) : null}
      <div className={cn("flex-1", dim && "opacity-(--opacity-dimmed)")} aria-busy={dim}>
        {failed ? (
          // The reason is in the core's log; the operator can do nothing with it here (design principle 13).
          <p className="px-xl py-lg text-body text-muted-foreground" data-phone-missions-state="unreadable">{t("missionTool.unreadable")}</p>
        ) : entries && entries.length === 0 ? (
          <div className="flex flex-col gap-xs px-xl py-xxl text-center" data-phone-missions-state="empty">
            <p className="text-title text-muted-foreground">{t("mobile.missions.empty")}</p>
            <p className="text-body text-subtle-foreground">{t("mobile.missions.start")}</p>
          </div>
        ) : (
          groupMissions(entries ?? []).map(({ group, entries: rows }) => <Group key={group} group={group} entries={rows} />)
        )}
        {!failed && omitted > 0 ? (
          <p className="px-xl py-md text-body text-muted-foreground" data-phone-missions-omitted={omitted}>{t("mobile.missions.omitted", { count: omitted })}</p>
        ) : null}
      </div>
      <TabBar active="missions" />
    </main>
  );
}

/** One group (B31): its name and count, its rows; 끝남 shows its last three until `모두 ›`. */
function Group({ group, entries }: { group: MissionGroup; entries: MissionEntry<PhoneMission>[] }) {
  const { t } = useInterfaceTranslation();
  const all = usePhone((s) => s.missionFolds.closed === true);
  const closed = group === "closed";
  const shown = closed && !all ? entries.slice(0, CLOSED_SHOWN) : entries;
  return (
    <section className="px-xl pt-lg" aria-label={`${t(`missionTool.group.${group}`)} ${entries.length}`} data-phone-mission-group={group} data-phone-mission-group-count={entries.length}>
      <h2 className={cn("flex items-center gap-sm pb-xs text-body", group === "turn" ? "font-semibold text-mission-hold" : "text-muted-foreground")}>
        <span>{t(`missionTool.group.${group}`)}</span>
        <span className="font-mono font-normal">{entries.length}</span>
        {closed && !all && entries.length > CLOSED_SHOWN ? (
          <Button variant="text" size="inline" className="ml-auto min-h-(--size-touch-target) gap-0" data-phone-fold="closed" onClick={() => setMissionFold("closed", true)}>
            {t("missionTool.all")}
            <ChevronRightIcon aria-hidden="true" className="size-(--size-icon)" />
          </Button>
        ) : null}
      </h2>
      <ul className="divide-y divide-border border-b border-border">
        {shown.map((entry) => (
          <li key={entry.mission.id}>{closed ? <ClosedRow mission={entry.mission} /> : <OpenRow entry={entry} />}</li>
        ))}
      </ul>
    </section>
  );
}

/**
 * An open mission's row (B31): its state, title, one line of what waits, works
 * or merges, a cell per step with `n/m`, and its time; a mission whose device
 * does not answer keeps its last values dimmed and counts no time.
 */
function OpenRow({ entry }: { entry: MissionEntry<PhoneMission> }) {
  const { t } = useInterfaceTranslation();
  const { mission, group, turn } = entry;
  const { done, total } = progress(mission);
  const elapsed = useElapsed(mission.stale ? null : (mission.started_at_unix_ms ?? mission.created_at_unix_ms));
  const first = turn[0] ?? null;
  const line = rowLine(mission, first);
  const text = line ? rowLineText(line, t) : null;
  const pill = t(missionPillKey(mission, first));
  return (
    <RowButton
      aria-label={t("missionTool.open", { title: mission.title })}
      data-phone-mission={mission.id}
      data-phone-mission-state={group}
      className={cn("min-h-(--size-touch-target) flex-col items-stretch gap-xs rounded-none px-none py-md hover:bg-transparent active:bg-accent", mission.stale && "opacity-(--opacity-dimmed)")}
      onClick={() => openMission(mission.id)}
    >
      <span className="flex w-full min-w-0 items-center gap-sm">
        <span
          className={cn("flex shrink-0 items-center rounded-full px-sm text-caption font-semibold", first ? PILL_TONE.hold : STATE_PILL_TONE[mission.status])}
          data-phone-mission-pill={first?.kind ?? mission.status}
        >
          {pill}
        </span>
        <span className="min-w-0 flex-1 break-keep text-title font-medium text-foreground wrap-anywhere">{mission.title}</span>
        <ChevronRightIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-muted-foreground" />
      </span>
      {text ? <span className="line-clamp-2 break-keep text-body text-subtle-foreground wrap-anywhere" data-phone-mission-line={line?.kind}>{text}</span> : null}
      <span className="flex min-w-0 flex-wrap items-center gap-sm text-body text-muted-foreground">
        {mission.nodes.length > 0 ? (
          <>
            <ProgressCells tones={missionCells(mission)} size="chip" />
            <span className="shrink-0 font-mono" data-phone-mission-counts={`${done}/${total}`}>{done}/{total}</span>
          </>
        ) : null}
        {elapsed ? <span className="shrink-0 font-mono">· {elapsed}</span> : null}
        {mission.device_label ? <DeviceTag label={mission.device_label} /> : null}
      </span>
    </RowButton>
  );
}

/** A finished or cancelled mission (B31): ✓ or a grey ✕, its title and the day it closed. */
function ClosedRow({ mission }: { mission: PhoneMission }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const closed = mission.closed_at_unix_ms;
  const done = mission.status === "done";
  return (
    <RowButton
      aria-label={t("missionTool.open", { title: mission.title })}
      data-phone-mission={mission.id}
      data-phone-mission-state="closed"
      className="min-h-(--size-touch-target) gap-sm rounded-none px-none py-md text-body hover:bg-transparent active:bg-accent"
      onClick={() => openMission(mission.id)}
    >
      {done ? <CheckIcon aria-label={t("mission.pill.done")} className="size-(--size-icon) shrink-0 text-mission-done" /> : <XIcon aria-label={t("mission.pill.cancelled")} className="size-(--size-icon) shrink-0 text-muted-foreground" />}
      <span className="min-w-0 flex-1 break-keep text-subtle-foreground wrap-anywhere">{mission.title}</span>
      {mission.device_label ? <DeviceTag label={mission.device_label} /> : null}
      {closed != null ? <span className="shrink-0 font-mono text-muted-foreground">{formatDateTime(language, closed, { month: "numeric", day: "numeric" })}</span> : null}
    </RowButton>
  );
}
