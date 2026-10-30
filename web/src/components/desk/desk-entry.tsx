import { DiamondIcon } from "lucide-react";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import { StatusMark } from "../status-mark";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { RowButton } from "../ui/row-button";
import { Hint } from "../ui/tooltip";

/**
 * The sidebar's `◆ Desk` row at its top (D-21, D-38, B1, B3): the only place
 * ◆ is drawn; Desk's own tabs inside a checkout's ◆ Desk group carry none.
 * A thread asking the operator puts the warning mark at its right.
 */
export function DeskSidebarRow({ selected, asking, onOpen }: { selected: boolean; asking: number; onOpen: () => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <RowButton className={cn("h-(--size-sidebar-line) rounded-sm px-xs text-body", selected ? "bg-sidebar-accent text-sidebar-accent-foreground" : "text-sidebar-foreground")} aria-current={selected ? "page" : undefined} onClick={onOpen} data-desk-sidebar-row="true">
      <DiamondIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 fill-current text-subtle-foreground" />
      <span className="min-w-0 flex-1 truncate font-medium">{t("desk.name")}</span>
      {asking > 0 ? <StatusMark mark="turn" label={t("desk.lane.threadsCount", { count: asking })} /> : null}
    </RowButton>
  );
}

/**
 * At the head of a thread's Agent tab (D-42, B35, B36): `나도 참여 중` while
 * the operator has typed into it, and the way back to where Desk was.
 */
export function DeskPaneReturn({ joined, onDesk }: { joined: boolean; onDesk: () => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <span className="flex shrink-0 items-center gap-xs" data-desk-pane-return="true">
      {joined ? <Badge variant="outline" data-desk-job-joined="true">{t("desk.job.joined")}</Badge> : null}
      <Hint label={t("desk.pane.toDesk")} shortcut="⌘J">
        <Button variant="ghost" size="sm" className="h-(--size-control-compact) gap-xxs px-xs text-caption font-normal text-subtle-foreground" onClick={onDesk}>
          <DiamondIcon aria-hidden="true" className="size-(--size-icon-sm) fill-current" />
          {t("desk.pane.toDesk")}
        </Button>
      </Hint>
    </span>
  );
}
