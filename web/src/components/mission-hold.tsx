import { PauseIcon } from "lucide-react";
import { useInterfaceTranslation } from "../i18n/client";
import { missionDelay } from "../missions";
import type { Mission } from "../snapshot";
import { useRemaining } from "./elapsed";

/** A mission delay as a chip says it: its reason, and when it ends by itself. */
export type MissionDelay = { line: string; until: number | null };

/** The delay an open mission is held by (`missionDelay`) with its end. */
export function heldBy(mission: Mission): MissionDelay | null {
  const line = missionDelay(mission);
  return line ? { line, until: mission.delay_until_unix_ms ?? null } : null;
}

/**
 * The words a held mission's state pill says in place of `진행 중`:
 * `보류 · 40분` while its delay holds, `보류` once its end has passed and
 * the core has not lifted it yet. None while nothing holds the mission.
 */
export function useDelayPill(mission: Mission): string | null {
  const { t } = useInterfaceTranslation();
  const delay = heldBy(mission);
  const left = useRemaining(delay?.until);
  if (!delay) return null;
  return left && left !== "past" ? t("mission.pill.heldLeft", { left: left.text }) : t("mission.pill.held");
}

/** What a chip's name says of a mission delay: `새 작업 보류 · <why> · 40분`, the time gone once its end has passed. */
export function useDelayWords(delay: MissionDelay | null): string | null {
  const { t } = useInterfaceTranslation();
  const left = useRemaining(delay?.until);
  if (!delay) return null;
  return left && left !== "past" ? t("mission.delayLeft", { line: delay.line, left: left.text }) : t("mission.delay", { line: delay.line });
}

/** The pause mark a held pill leads with. */
export function HeldMark() {
  return <PauseIcon aria-hidden="true" className="size-(--size-icon-sm)" />;
}
