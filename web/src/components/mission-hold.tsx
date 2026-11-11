import { PauseIcon } from "lucide-react";
import { useInterfaceTranslation } from "../i18n/client";
import { missionDelay } from "../missions";
import type { Mission } from "../snapshot";
import { useRemaining } from "./elapsed";

/**
 * The words a held mission's state pill says in place of `진행 중`:
 * `보류 · 40분 남음` while its delay holds, `보류` once its end has passed and
 * the core has not lifted it yet. None while nothing holds the mission.
 */
export function useDelayPill(mission: Mission): string | null {
  const { t } = useInterfaceTranslation();
  const delay = missionDelay(mission);
  const left = useRemaining(delay ? mission.delay_until_unix_ms : null);
  if (!delay) return null;
  return left && left !== "past" ? t("mission.pill.heldLeft", { left: left.text }) : t("mission.pill.held");
}

/** The pause mark a held pill leads with. */
export function HeldMark() {
  return <PauseIcon aria-hidden="true" className="size-(--size-icon-sm)" />;
}
