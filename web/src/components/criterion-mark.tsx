import type { MissionCriterionState } from "../snapshot";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { Hint } from "./ui/tooltip";

// A finish line's state as one mark (PRD mission-map-git-criteria B32): ✓
// passed, ○ open, ⤳ skipped, ✕ failed, the same in the criteria list and on
// the CI row. The hover and the accessible name say the same state word.

const MARK: Record<MissionCriterionState, { glyph: string; tone: string }> = {
  pass: { glyph: "✓", tone: "text-muted-foreground" },
  open: { glyph: "○", tone: "text-muted-foreground" },
  skipped: { glyph: "⤳", tone: "text-mission-hold" },
  fail: { glyph: "✕", tone: "text-mission-failed" },
};

export function CriterionMark({ state, className }: { state: MissionCriterionState; className?: string }) {
  const { t } = useInterfaceTranslation();
  const word = t(`mission.lineState.${state}`);
  const mark = MARK[state];
  return (
    <Hint label={word}>
      <span role="img" aria-label={word} data-criterion-mark={state} className={cn("shrink-0 font-semibold", mark.tone, className)}>
        <span aria-hidden="true">{mark.glyph}</span>
      </span>
    </Hint>
  );
}
