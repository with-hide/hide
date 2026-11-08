import type { Actions } from "../actions";
import { humanTurnText } from "../askSubject";
import { useUnreachable } from "../deviceReach";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { machineName } from "../screenMachine";
import { useShellStore } from "../store";
import type { HumanKind } from "../snapshot";
import { DeviceChip } from "./device-chip";
import { Button } from "./ui/button";

/** What Done needs of a person's turn: the declaration it settles, what it says, and the device its agent runs on (absent for this machine). */
export type HumanTurnTarget = { id: string; kind: HumanKind; place: string; device_id?: string | null };

/**
 * Done on a person's turn (PRD ask-human D-10, B18, B19, B21): one part on
 * every surface that shows the turn, the two lists' Needs You, the pane's
 * head, the Overview and the tree. A press sends the agent one letter
 * through the core and the turn goes; while the agent's device is not
 * connected it is disabled with the reason beside it, and the turn stays.
 * A row draws it above its open button (`relative z-10`), so a press never
 * opens the row too.
 */
export function HumanTurnDone({ turn, actions, className }: { turn: HumanTurnTarget; actions: Pick<Actions, "finishHumanTurn">; className?: string }) {
  const { t } = useInterfaceTranslation();
  const unreachable = useUnreachable({ device: turn.device_id ?? null });
  const what = humanTurnText(t, turn);
  return (
    <span className={cn("relative z-10 flex min-w-0 items-center gap-xs", className)} data-human-turn-done-slot={turn.id}>
      <Button
        type="button"
        variant="secondary"
        size="sm"
        disabled={unreachable !== null}
        aria-label={t("agents.human.doneName", { what })}
        data-human-turn-done={turn.id}
        className="shrink-0"
        onMouseDown={(event) => event.preventDefault()}
        onClick={(event) => {
          event.stopPropagation();
          actions.finishHumanTurn(turn.id);
        }}
      >
        {t("agents.human.done")}
      </Button>
      {unreachable ? <span className="min-w-0 truncate text-caption text-muted-foreground" title={unreachable} data-human-turn-unreachable={turn.id}>{unreachable}</span> : null}
    </span>
  );
}

/**
 * A person's turn as a line where no ask line stands for it, such as the
 * declaring child's tree row (PRD ask-human B14): its kind and place as
 * plain text the operator can select, never a link (B10), and the chip of
 * the device it waits on when that is not this machine (B9).
 */
export function HumanTurnLine({ turn, className }: { turn: HumanTurnTarget; className?: string }) {
  const { t } = useInterfaceTranslation();
  const device = useShellStore((state) => (turn.device_id ? machineName(state.rest?.navigator?.devices, turn.device_id, t) : null));
  const what = humanTurnText(t, turn);
  return (
    <span className={cn("flex min-w-0 items-baseline gap-xs text-caption", className)} data-human-turn-line={turn.id}>
      <span className="pointer-events-auto min-w-0 cursor-text truncate text-warning select-text" title={what} data-human-turn-place={turn.id}>{what}</span>
      {device ? <DeviceChip label={device} variant="inline" className="shrink-0 self-center" /> : null}
    </span>
  );
}
