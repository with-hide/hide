import { DiamondIcon } from "lucide-react";
import { useInterfaceTranslation } from "../../i18n/client";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { Hint } from "../ui/tooltip";

/**
 * At the head of a thread's Agent tab (D-42, B35, B36): `나도 참여 중` while
 * the operator has typed into it, and the way back to where Desk was.
 */
export function DeskPaneReturn({ joined, chord = "⌘J", onDesk }: { joined: boolean; chord?: string | null; onDesk: () => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <span className="flex shrink-0 items-center gap-xs" data-desk-pane-return="true">
      {joined ? <Badge variant="outline" data-desk-job-joined="true">{t("desk.job.joined")}</Badge> : null}
      <Hint label={t("desk.pane.toDesk")} shortcut={chord ?? undefined}>
        <Button variant="ghost" size="sm" className="h-(--size-control-compact) gap-xxs px-xs text-caption font-normal text-subtle-foreground" data-desk-pane-to-desk="true" onClick={onDesk}>
          <DiamondIcon aria-hidden="true" className="size-(--size-icon-sm) fill-current" />
          {t("desk.pane.toDesk")}
        </Button>
      </Hint>
    </span>
  );
}
