import type { Actions } from "../actions";
import { humanTurnText } from "../askSubject";
import { useUnreachable } from "../deviceReach";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { machineName } from "../screenMachine";
import { useShellStore } from "../store";
import type { HumanKind } from "../snapshot";
import { DeviceChip } from "./device-chip";
import { HumanTurnCopy, type CopyText } from "./human-turn-copy";
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
 * opens the row too. A surface on the secondary fill, the pane's band, takes
 * `outline` so the button stands out from it, and one that already says why
 * the device is away leaves `reason` off.
 */
export function HumanTurnDone({ turn, actions, variant = "secondary", reason = true, className }: { turn: HumanTurnTarget; actions: Pick<Actions, "finishHumanTurn">; variant?: "secondary" | "outline"; reason?: boolean; className?: string }) {
  const { t } = useInterfaceTranslation();
  const unreachable = useUnreachable({ device: turn.device_id ?? null });
  const what = humanTurnText(t, turn);
  return (
    <span className={cn("relative z-10 flex min-w-0 items-center gap-xs", className)} data-human-turn-done-slot={turn.id}>
      <Button
        type="button"
        variant={variant}
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
      {unreachable && reason ? <span className="min-w-0 truncate text-caption text-muted-foreground" title={unreachable} data-human-turn-unreachable={turn.id}>{unreachable}</span> : null}
    </span>
  );
}

/**
 * A person's turn as a line where no ask line stands for it, such as the
 * declaring child's tree row (PRD ask-human B14): its kind and place as
 * plain text the operator can select, never a link (B10), Copy beside the
 * place where the surface draws Done (`copy`), and the chip of the device it
 * waits on when that is not this machine (B9). `textHidden` keeps the words
 * from assistive technology on a row whose open button already says them.
 */
export function HumanTurnLine({ turn, copy, textHidden = false, className }: { turn: HumanTurnTarget; copy?: CopyText; textHidden?: boolean; className?: string }) {
  const { t } = useInterfaceTranslation();
  const device = useShellStore((state) => (turn.device_id ? machineName(state.rest?.navigator?.devices, turn.device_id, t) : null));
  const what = humanTurnText(t, turn);
  return (
    <span className={cn("flex min-w-0 items-baseline gap-xs text-caption", className)} data-human-turn-line={turn.id}>
      <span aria-hidden={textHidden || undefined} className="pointer-events-auto min-w-0 cursor-text truncate text-warning select-text" title={what} data-human-turn-place={turn.id}>{what}</span>
      {copy ? <HumanTurnCopy turn={turn} copy={copy} /> : null}
      {device ? <span aria-hidden={textHidden || undefined} className="contents"><DeviceChip label={device} variant="inline" className="shrink-0 self-center" /></span> : null}
    </span>
  );
}
