import type { Actions } from "./actions";
import { Button } from "./components/ui/button";
import { useInterfaceTranslation } from "./i18n/client";
import { useUiStore } from "./ui";

/**
 * A Workspace or agent asked for from a screen other than the Workspace (All
 * projects, an Overview, the mission screen, Desk) that is not in front yet:
 * a quiet line while it is on its way, and the refusal with Dismiss when it
 * was not brought forward, so the operator stays where they were and knows
 * why (B2, B21). `CenterScreen` draws it above whichever of those screens is
 * in front; on the Workspace the core's notice says it instead.
 */
export function OpeningStatus({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const opening = useUiStore((s) => s.opening);
  if (!opening) return null;
  if (!opening.failure) {
    return (
      <p role="status" className="shrink-0 px-md pt-sm text-caption text-muted-foreground" data-opening="pending">
        {t("overview.opening")}
      </p>
    );
  }
  return (
    <div role="alert" className="mx-md mt-sm flex shrink-0 items-center gap-sm rounded-sm bg-card px-sm py-xs text-caption text-warning" data-opening="failed">
      <span className="min-w-0 flex-1 break-words">{t("overview.notOpened", { reason: opening.failure })}</span>
      <Button variant="secondary" onClick={() => actions.dismissOpening()} data-opening-dismiss="true">
        {t("workspace.dismiss")}
      </Button>
    </div>
  );
}
