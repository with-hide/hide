import type { WorkingPhase } from "../../desk";
import type { MessageKey } from "../../i18n/catalogs";
import { useInterfaceTranslation } from "../../i18n/client";
import { Elapsed } from "../elapsed";

const PHASE_WORDS = {
  thinking: "desk.working.thinking",
  answering: "desk.working.answering",
  sessions: "desk.working.sessions",
  files: "desk.working.files",
  editing: "desk.working.editing",
  letters: "desk.working.letters",
  starts: "desk.working.starts",
  other: "desk.working.other",
} as const satisfies Record<WorkingPhase, MessageKey>;

/**
 * The one line that says an agent is working (feedback 1 c3), at the foot of
 * Desk's conversation and of a session's thread alike: what it does now in a
 * short word from a closed set, then how long the turn has run. Only the word
 * moves, a slow shimmer (`.working-shimmer`), still under reduced motion; the
 * time stays still. Its height is fixed, so the conversation does not jump
 * when it goes. The caller draws it from the moment a turn starts until it
 * ends, and not while the turn waits on the operator.
 */
export function WorkingLine({ phase, since }: { phase: WorkingPhase; since: number | null }) {
  const { t } = useInterfaceTranslation();
  return (
    <p role="status" className="flex h-(--size-control-compact) shrink-0 items-center gap-xxs text-caption text-muted-foreground" data-working-line={phase}>
      <span className="working-shimmer">{t(PHASE_WORDS[phase])}</span>
      {since !== null ? <><span aria-hidden="true">·</span><Elapsed since={since} /></> : null}
    </p>
  );
}
