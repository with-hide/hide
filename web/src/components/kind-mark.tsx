import { ListChecksIcon, WaypointsIcon } from "lucide-react";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";

// What kind of work a page shows: a mission's agents work side by side, a
// task's one agent works down its own steps. The kind is a word with its
// mark in a colour that is no state's (`--kind-mission`, `--kind-task`), in
// the head of a mission's page, a plan's page and Settings › Mission; the
// `?` beside it, which opens the window telling the two apart, is
// `KindHelpButton` (`kind-help-dialog.tsx`).

export type WorkKind = "mission" | "task";

const LOOK: Record<WorkKind, { Icon: typeof WaypointsIcon; tone: string }> = {
  mission: { Icon: WaypointsIcon, tone: "bg-kind-mission/15 text-kind-mission" },
  task: { Icon: ListChecksIcon, tone: "bg-kind-task/15 text-kind-task" },
};

/** The kind's mark and word: `미션` with the connected nodes, `태스크` with the checklist. */
export function KindMark({ kind, className }: { kind: WorkKind; className?: string }) {
  const { t } = useInterfaceTranslation();
  const { Icon, tone } = LOOK[kind];
  return (
    <span className={cn("inline-flex shrink-0 items-center gap-xxs rounded-xs px-xs py-xxs text-micro font-semibold leading-none", tone, className)} data-kind-mark={kind}>
      <Icon aria-hidden="true" className="size-(--size-icon-sm)" />
      {t(`kind.${kind}`)}
    </span>
  );
}
