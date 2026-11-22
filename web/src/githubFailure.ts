import type { TFunction } from "i18next";
import type { MessageKey } from "./i18n/catalogs";
import { githubReasonsEnglish } from "./i18n/resources/githubReasons";
import { isOtherHost, namedHost } from "./codeHost";
import type { GithubFailureCategory, GithubStatus } from "./snapshot";

/** A refusal of `gh` itself rather than of one repository: missing, older than Hide's commands need, or signed out. It is the same for every repository the core reads, so it speaks before any other failure. */
export type GhRefusal = "not_installed" | "outdated" | "not_logged_in";

export function isGhRefusal(category: GithubFailureCategory | null | undefined): category is GhRefusal {
  return category === "not_installed" || category === "outdated" || category === "not_logged_in";
}

/** The one line each refusal shows wherever GitHub cannot be read because of it. */
export const GH_REFUSAL_KEY: Record<GhRefusal, MessageKey> = {
  not_installed: "issueSettings.ghNotInstalled",
  outdated: "issueSettings.ghOutdated",
  not_logged_in: "issueSettings.ghNotLoggedIn",
};

/**
 * A host read's failure category in the viewer's language. A GitHub status keeps GitHub's own lines; another host's
 * names its program for a refusal of the program and the host otherwise. One this build does not word reads as the
 * general line.
 */
export function githubFailureText(status: Pick<GithubStatus, "failure_category" | "host">, t: TFunction<"translation">): string {
  const category = status.failure_category ?? "other";
  const host = namedHost(status.host);
  if (isOtherHost(status.host)) {
    if (category !== "other" && isGhRefusal(category)) return t(GH_REFUSAL_KEY[category], { program: host.program });
    return t(category === "network_or_rate_limit" ? "githubReason.hostUnanswered" : "githubReason.hostUnread", { host: host.name });
  }
  const key = `githubReason.${category}`;
  return t(key in githubReasonsEnglish ? (key as MessageKey) : "githubReason.other");
}
