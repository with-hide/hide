import type { Actions } from "./actions";
import { Button } from "./components/ui/button";
import { windowStrip } from "./connection";
import { useInterfaceTranslation } from "./i18n/client";
import { coreMachineName, moveMachines } from "./screenMachine";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { cn } from "./lib/utils";

export function ConnectionBadge({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const connection = useShellStore((s) => s.connection);
  const refused = useShellStore((s) => s.refused);
  const move = useShellStore((s) => s.coreMove);
  const link = useShellStore((s) => s.coreLink);
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const strip = windowStrip({ connection, refused, move, link, machines: move ? moveMachines(devices, move, t) : null, coreMachine: coreMachineName(devices, t) }, t);
  if (!strip) return null;
  return (
    <div
      role="status"
      className="flex items-center gap-xs border-b border-border bg-card px-md py-xs text-caption text-subtle-foreground"
      data-connection={strip.kind}
    >
      {strip.mark ? (
        <span aria-hidden="true" className={cn("shrink-0", strip.mark === "warn" ? "font-semibold text-warning" : "text-muted-foreground")}>
          {strip.mark === "warn" ? "!" : "…"}
        </span>
      ) : null}
      <span className="min-w-0 truncate">{strip.text}</span>
      {strip.action ? <span aria-hidden="true">·</span> : null}
      {strip.action === "reconnect" ? (
        <Button variant="link" size="sm" className="h-auto shrink-0 p-0 text-caption" data-core-reconnect="true" onClick={() => actions.coreLink("reconnect")}>
          {t("coreMove.reconnect")}
        </Button>
      ) : null}
      {strip.action === "retry" ? (
        <Button variant="link" size="sm" className="h-auto shrink-0 p-0 text-caption" data-core-retry="true" onClick={() => actions.coreLink("retry")}>
          {t("common.retry")}
        </Button>
      ) : null}
      {strip.action === "give_up" ? (
        // It only opens the move dialog at its confirm; nothing is given up from the strip.
        <Button variant="link" size="sm" className="h-auto shrink-0 p-0 text-caption" data-core-move-strip-give-up="true" onClick={() => useUiStore.getState().openMoveGiveUp()}>
          {move?.direction === "back" ? t("coreMove.takeHere.take") : t("coreMove.giveUp.stop")}
        </Button>
      ) : null}
    </div>
  );
}
