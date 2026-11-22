// The line under a sleeping agent's row while it waits (PRD core-wait D-15,
// B18): what it waits on in a few words, `#12 CI 기다림`, led under a day
// before its deadline by the hours left, which the narrow row must not cut. Pure, so the sidebar row, the sleeping
// session row and their tests read one value.

import type { TFunction } from "i18next";
import type { SleepWait } from "./snapshot";

const DAY_MS = 24 * 60 * 60 * 1000;
const HOUR_MS = 60 * 60 * 1000;

export function sleepWaitWords(wait: SleepWait, now: number, t: TFunction<"translation">): string {
  const target = wait.target;
  // A pull request is named as its code host writes it (`#12`, `!12`).
  const pr = (number: number) => wait.reference?.label || `#${number}`;
  const what = "ci" in target ? t("agents.sleep.wait.ci", { pr: pr(target.ci) })
    : "pr" in target ? t("agents.sleep.wait.pr", { pr: pr(target.pr) })
      : "branch" in target ? t("agents.sleep.wait.branch", { branch: target.branch })
        : "mission" in target ? t("agents.sleep.wait.mission", { mission: target.mission })
          : t("agents.sleep.wait.node", { mission: target.node.mission, node: target.node.node });
  const left = wait.until_unix_ms - now;
  return left > 0 && left < DAY_MS ? `${t("mission.wait.left", { hours: String(Math.max(1, Math.ceil(left / HOUR_MS))) })} · ${what}` : what;
}
