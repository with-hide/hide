// A screen's one-line introduction (PRD first-run-onboarding D-18, B43): what
// the screen does and its first step, shown the first time the screen opens
// and never again; today the Factory's alone, since Settings > Devices
// already says what it is for. The core keeps which screens were introduced
// (`ui_state.screen_intros_seen`), so another window or a restart does not
// show it twice; this visit keeps the line it opened with.

import { BrandOwl } from "./components/brand-owl";
import { useEffect, useState } from "react";
import type { Actions } from "./actions";
import { useInterfaceTranslation } from "./i18n/client";
import type { MessageKey } from "./i18n/catalogs";
import type { ScreenIntro as Screen } from "./snapshot";
import { useShellStore } from "./store";

const WORDS: Record<Screen, MessageKey> = {
  factory: "firstRun.intro.factory",
};

/**
 * Whether this visit shows `screen`'s introduction: decided once, when the
 * first snapshot says whether it was seen, and kept for the visit, so the
 * screen calls it where it stays mounted and draws `ScreenIntroLine` where
 * its layout wants the line.
 */
export function useScreenIntro(screen: Screen): boolean {
  // Unknown until the first snapshot says which screens were introduced.
  const seen = useShellStore((s) => (s.rest?.ui_state ? (s.rest.ui_state.screen_intros_seen ?? []).includes(screen) : null));
  const [shown, setShown] = useState<boolean | null>(null);
  useEffect(() => {
    if (shown === null && seen !== null) setShown(!seen);
  }, [seen, shown]);
  return shown === true;
}

/**
 * The line itself, under the screen's title. It records the screen as
 * introduced once it has been drawn, so a visit that never reaches the line
 * (still loading, or straight to a Task page) leaves it for the next one (B43).
 */
export function ScreenIntroLine({ screen, actions }: { screen: Screen; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  useEffect(() => {
    actions.markScreenIntroSeen(screen);
  }, [actions, screen]);
  return (
    <p className="mx-lg mb-md flex items-center gap-sm rounded-md bg-card px-md py-sm text-body text-subtle-foreground" data-screen-intro={screen}>
      <BrandOwl pose="factory" className="size-(--size-owl-intro)" />
      {t(WORDS[screen])}
    </p>
  );
}
