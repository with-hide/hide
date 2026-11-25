// The phone's bottom bar (PRD mobile-companion-ui B28): the places a phone
// has, each with its icon, its name and, when something waits on the
// operator, its count. A new place is one more entry of `PHONE_TABS` and a
// count in `useBadges`; the bar is drawn on the lists only, never over a
// detail.

import { GitPullRequestIcon, ListIcon, RepeatIcon, SettingsIcon, WaypointsIcon, type LucideIcon } from "lucide-react";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import type { MessageKey } from "../i18n/catalogs";
import { useInterfaceTranslation } from "../i18n/translator";
import { useMissionBadge } from "./missionModel";
import { setTab, usePhone, type PhoneTab } from "./store";

export const PHONE_TABS: readonly { id: PhoneTab; icon: LucideIcon; label: MessageKey }[] = [
  { id: "agents", icon: ListIcon, label: "mobile.tab.agents" },
  { id: "missions", icon: WaypointsIcon, label: "mobile.tab.missions" },
  { id: "automations", icon: RepeatIcon, label: "mobile.tab.automations" },
  { id: "prs", icon: GitPullRequestIcon, label: "mobile.tab.prs" },
  { id: "settings", icon: SettingsIcon, label: "mobile.tab.settings" },
];

/**
 * What waits on the operator in each place: the agent list's 내 차례, its rows, the automations that stopped for a person (each its own
 * line there, PRD automations D-12, B23) and the missions that are their own item, the number the desktop's 내 차례 says; and the missions
 * with anything in theirs; the automations, PR and Settings tabs have no count (B28).
 */
function useBadges(): Record<PhoneTab, number> {
  const agents = usePhone((s) => (s.groups?.find((group) => group.group === "needs_you")?.count ?? 0) + s.automationAsks.length + s.missionAsks.length);
  const missions = useMissionBadge();
  return { agents, missions, automations: 0, prs: 0, settings: 0 };
}

export function TabBar({ active }: { active: PhoneTab }) {
  const { t } = useInterfaceTranslation();
  const badges = useBadges();
  // A Hide before automations sends no `automations` frame, so its tab stays away (docs/ARCHITECTURE.md, an addition keeps the number).
  const automations = usePhone((s) => s.automations !== null);
  const tabs = PHONE_TABS.filter(({ id }) => id !== "automations" || automations);
  return (
    <nav aria-label={t("mobile.tabs")} className="phone-safe-bottom sticky bottom-0 mt-auto grid grid-flow-col auto-cols-fr border-t border-border bg-background" data-phone-tabs={active}>
      {tabs.map(({ id, icon: Icon, label }) => {
        const count = badges[id];
        const here = id === active;
        return (
          <Button
            key={id}
            variant="ghost"
            aria-current={here ? "page" : undefined}
            aria-label={count > 0 ? `${t(label)} ${count}` : t(label)}
            data-phone-tab={id}
            data-phone-tab-count={count > 0 ? String(count) : undefined}
            className={`h-auto min-h-(--size-touch-target) min-w-0 flex-col gap-xxs rounded-none px-xxs py-sm text-caption ${here ? "text-foreground" : "text-muted-foreground"}`}
            onClick={() => setTab(id)}
          >
            <span className="relative">
              <Icon aria-hidden="true" className="size-(--size-icon-lg)" />
              {count > 0 ? (
                <Badge aria-hidden="true" className="absolute -right-md -top-xs min-w-(--size-badge-height) rounded-full bg-mission-hold px-xs font-mono text-background">{count}</Badge>
              ) : null}
            </span>
            {/* Five places share a phone's width; a long word at a large text size ends in an ellipsis rather than meeting the next. */}
            <span className="max-w-full truncate">{t(label)}</span>
          </Button>
        );
      })}
    </nav>
  );
}
