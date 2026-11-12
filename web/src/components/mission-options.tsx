// A mission map head's destination tag and options line (PRD
// mission-destination D-18, D-21, D-50, B1, B48, B53): `→ <branch>` in the
// lime of where results land, then one grey line of what the mission applies,
// a value this mission changed in bright words with a small dot, never in the
// orange of a planned human step. Hovering or focusing the line (pressing it
// on the phone) shows each value, where it comes from, why when known, and
// how to change it; nothing on it is pressed to change anything.

import { Fragment } from "react";
import { useInterfaceTranslation } from "../i18n/translator";
import { cn } from "../lib/utils";
import type { MissionOption } from "../missionOptions";
import { Hint, Tooltip, TooltipContent, TooltipTrigger, useHintOpen } from "./ui/tooltip";

/**
 * The branch the mission's results go to, cut with `…` when long and whole on
 * hover (B30). On the phone (`touch`), which has no hover and no popover
 * (D-38), the tag names itself to a screen reader instead of hinting.
 */
export function MissionDestinationTag({ branch, touch = false, className }: { branch: string; touch?: boolean; className?: string }) {
  const { t } = useInterfaceTranslation();
  const label = t("mission.head.destination", { branch });
  const tag = (
    <span
      className={cn("flex min-w-0 max-w-(--size-flow-title) shrink items-center gap-xxs rounded-xs border border-mission-land/40 px-xs font-mono text-caption text-mission-land", className)}
      data-mission-destination={branch}
      {...(touch ? { role: "note", "aria-label": label } : {})}
    >
      <span aria-hidden="true" className="shrink-0">→</span>
      <span className="min-w-0 truncate">{branch}</span>
    </span>
  );
  return touch ? tag : <Hint label={label}>{tag}</Hint>;
}

/** The options as one line of words, joined by `·`; the words of a changed value bright with its dot. */
export function OptionWords({ options }: { options: readonly MissionOption[] }) {
  return (
    <>
      {options.map((option, index) => (
        <Fragment key={option.key}>
          {index > 0 ? <span aria-hidden="true"> · </span> : null}
          <span className={cn("whitespace-nowrap", option.changed && "font-medium text-foreground")} data-mission-head-option={option.key} data-mission-head-option-changed={option.changed || undefined}>
            {option.short}
            {option.changed ? <span aria-hidden="true" className="ml-xxs inline-block size-xs rounded-full bg-foreground align-middle" data-mission-head-option-dot="true" /> : null}
          </span>
        </Fragment>
      ))}
    </>
  );
}

/** Each option in full, as the line's hover and the phone's pressed line show it. */
export function OptionTable({ options }: { options: readonly MissionOption[] }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex min-w-0 flex-col gap-sm text-caption" data-mission-options-detail="true">
      <dl className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)_auto] gap-x-sm gap-y-xxs">
        {options.map((option) => (
          <Fragment key={option.key}>
            <dt className="text-muted-foreground">{t(`mission.options.item.${option.key}`)}</dt>
            <dd className="min-w-0 break-words text-foreground" data-mission-head-option-value={option.key}>
              {option.value}
              {option.why ? <span className="text-muted-foreground" data-mission-head-option-why={option.key}> · {option.why}</span> : null}
            </dd>
            <dd className={cn("text-right", option.changed ? "font-medium text-foreground" : "text-muted-foreground")} data-mission-head-option-source={option.changed ? "mission" : "settings"}>
              {t(option.changed ? "mission.options.source.mission" : "mission.options.source.settings")}
            </dd>
          </Fragment>
        ))}
      </dl>
      <p className="text-muted-foreground" data-mission-options-foot="true">{t("mission.options.foot")}</p>
    </div>
  );
}

/** The options line in the desktop map's head, its detail on hover or focus. */
export function MissionOptionsLine({ options, className }: { options: readonly MissionOption[]; className?: string }) {
  const { t } = useInterfaceTranslation();
  const { open, onOpenChange, triggerProps } = useHintOpen();
  const changed = options.filter((option) => option.changed).map((option) => option.short);
  const label = [t("mission.options.label"), options.map((option) => option.short).join(" · "), ...(changed.length > 0 ? [`${t("mission.options.changed")}: ${changed.join(", ")}`] : [])].join(" · ");
  return (
    <Tooltip open={open} onOpenChange={onOpenChange}>
      <TooltipTrigger asChild {...triggerProps}>
        <span tabIndex={0} role="note" aria-label={label} className={cn("min-w-0 truncate rounded-xs text-caption text-muted-foreground outline-none focus-visible:ring-1 focus-visible:ring-ring", className)} data-mission-options="true">
          <OptionWords options={options} />
        </span>
      </TooltipTrigger>
      <TooltipContent side="bottom" align="start" className="w-(--size-mission-popover) max-w-(--radix-tooltip-content-available-width) text-left text-wrap rounded-md p-md">
        <OptionTable options={options} />
      </TooltipContent>
    </Tooltip>
  );
}
