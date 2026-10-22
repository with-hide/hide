import { useElapsed } from "./components/elapsed";
import { useInterfaceTranslation } from "./i18n/client";

/** How current a value read from Git or GitHub is (`contracts/snapshot-wire-enums.json`: `observed_freshness`). */
export const FRESHNESS = ["fresh", "stale", "failed"] as const;
export type Freshness = (typeof FRESHNESS)[number];

/**
 * One shape for a value read from Git or GitHub (PRD git-freshness D-09): the last value read stays beside it,
 * with when it was read and whether it is current, older than its reader's interval, or behind a failed read.
 */
export type Observation = { freshness: Freshness; read_at_unix_ms: number | null };

/** The tooltip line of a value that is not current, in the PR list's words (`GitHub 읽기 실패 · N분 전 값`), or null. */
export function useObservedLine(observation: Observation | null | undefined, source: "git" | "github"): string | null {
  const { t } = useInterfaceTranslation();
  const elapsed = useElapsed(observation?.read_at_unix_ms);
  if (!observation || observation.freshness === "fresh") return null;
  if (observation.freshness === "stale") return elapsed ? t("checkoutGit.observed.stale", { time: elapsed }) : null;
  return elapsed ? t(`checkoutGit.observed.${source}Failed`, { time: elapsed }) : t(`checkoutGit.observed.${source}FailedNever`);
}
