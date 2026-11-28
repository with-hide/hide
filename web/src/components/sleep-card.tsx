import { CircleAlertIcon, MoonIcon } from "lucide-react";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { wakeFailureReason, wakingLine, type SleepView } from "../sleep";
import { Button } from "./ui/button";

/**
 * What a sleeping session says wherever it is drawn (PRD agent-sleep
 * B11-B14): in place of its pane's terminal, and in the middle of its
 * dimmed conversation, in the pane and beside Desk. Sleeping with the last
 * progress line and Wake agent; Waking… with how old the resumed
 * conversation is; or `Couldn’t resume this conversation` with the core's
 * reason, Retry and Start new session. `conversationSleep` (`sleep.ts`)
 * decides the state and which presses it offers.
 */
export function SleepCard({ paneId, view, className }: { paneId: string; view: SleepView; className?: string }) {
  const { t } = useInterfaceTranslation();
  const { sleep, onWake, onFresh } = view;
  return (
    <div className={cn("flex flex-col items-center gap-sm text-center", className)} data-pane-sleep={sleep.state}>
      {sleep.state === "sleeping" ? (
        <>
          <MoonIcon className="size-(--size-icon-lg) text-muted-foreground" aria-hidden="true" />
          <p className="text-subhead text-foreground">{t("panes.sleep.cardSleeping")}</p>
          {sleep.progress ? <p className="max-w-full truncate text-caption text-muted-foreground" data-pane-sleep-progress="true">{sleep.progress}</p> : null}
          {onWake ? (
            <Button variant="secondary" size="sm" className="mt-xs" data-agent-wake={paneId} onClick={onWake}>
              {t("panes.sleep.wake")}
            </Button>
          ) : null}
        </>
      ) : sleep.state === "waking" ? (
        <>
          <MoonIcon className="size-(--size-icon-lg) text-muted-foreground" aria-hidden="true" />
          <p className="text-subhead text-foreground" role="status">
            {t("panes.sleep.waking")}
          </p>
          <p className="text-caption text-muted-foreground">{wakingLine(sleep, Date.now(), t)}</p>
        </>
      ) : (
        <>
          <CircleAlertIcon className="size-(--size-icon-lg) text-destructive" aria-hidden="true" />
          <p className="text-subhead text-foreground">{t("panes.sleep.resumeFailed")}</p>
          {sleep.reason ? (
            <p className="max-w-full text-caption text-muted-foreground" role="alert" data-pane-sleep-reason="true">
              {wakeFailureReason(sleep, t)}
            </p>
          ) : null}
          {onWake || onFresh ? (
            <div className="mt-xs flex flex-wrap items-center justify-center gap-sm">
              {onWake ? (
                <Button size="sm" variant="secondary" data-agent-wake-retry={paneId} onClick={onWake}>
                  {t("common.retry")}
                </Button>
              ) : null}
              {onFresh ? (
                <Button size="sm" variant="outline" data-agent-wake-fresh={paneId} onClick={onFresh}>
                  {t("panes.sleep.newSession")}
                </Button>
              ) : null}
            </div>
          ) : null}
        </>
      )}
    </div>
  );
}
