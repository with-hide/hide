// The right half of ⌘K (PRD cmdk-navigation B15, B21): what the highlighted row
// is, said with the facts the snapshot carries and no others, and the
// relations it belongs to drawn the way the empty list draws them. A property
// with no value has no line (design principle 10); nothing here is computed
// that the core did not produce.

import type { TFunction } from "i18next";
import type { InterfaceLanguage } from "./i18n/locale";
import { relationRows, relationsOf, type RelationTarget, type Relations } from "./relations";
import type { EntryStatus, SearchEntry } from "./search";
import { lastReadWords } from "./searchGithub";
import { searchDevices } from "./search";
import { reviewWord } from "./prMark";
import type { LinkSummaries, SnapshotRest } from "./snapshot";
import { sameIssue } from "./linkPanel";
import { pullLabel } from "./codeHost";

export type Detail = {
  /** `Pull request`, `Agent · Claude`: the kind the title is of. */
  kind: string;
  title: string;
  pills: EntryStatus[];
  /** Plain pills: a device, a branch, a number. */
  tags: string[];
  facts: [string, string][];
  /** The relations of this row, when it has any beyond itself. */
  relations: Relations | null;
  /** What ↵ does. */
  action: string;
};


function provider(kind: string | undefined, t: TFunction<"translation">): string {
  if (!kind) return t("common.agent");
  return t("search.agentProvider", { provider: `${kind.charAt(0).toUpperCase()}${kind.slice(1)}` });
}

/** The relations target a row stands for: its own kind of thing, or null for one with no relations to draw. */
function targetOf(entry: SearchEntry): RelationTarget | null {
  if (entry.external) return null;
  if (entry.kind === "agent" && entry.paneId) return { kind: "agent", paneId: entry.paneId };
  if (entry.kind === "checkout" && entry.checkoutId) return { kind: "checkout", checkoutId: entry.checkoutId };
  if (entry.kind === "pr" && entry.workspaceId && entry.number !== undefined) return { kind: "pr", workspaceId: entry.workspaceId, number: entry.number };
  if (entry.kind === "issue" && entry.taskKey) return { kind: "issue", taskKey: entry.taskKey };
  return null;
}

function relationsFor(rest: SnapshotRest | null, entry: SearchEntry, t: TFunction<"translation">): Relations | null {
  const target = targetOf(entry);
  const relations = target ? relationsOf(rest, target, t) : null;
  // A relation of one row is no relation: the row itself is all it holds.
  return relations && relationRows(relations).length > 1 ? relations : null;
}

function present(facts: [string, string | null | undefined | false][]): [string, string][] {
  return facts.filter((row): row is [string, string] => typeof row[1] === "string" && row[1].length > 0);
}

/** The first line the agent last said, which is what its row's second line shows. */
function lastWords(entry: SearchEntry): string | null {
  const text = entry.agent?.detail || entry.agent?.message;
  return text ? (text.split("\n")[0] ?? null) : null;
}

/** `Sessions N` (PRD link-graph B35): the record's session lines for a PR or an issue, absent at zero. */
function sessionsFact(links: LinkSummaries | null, entry: SearchEntry, t: TFunction<"translation">): [string, string | null] {
  const summary = entry.workspace ? links?.projects[entry.workspace.id] : undefined;
  const count = entry.kind === "pr" ? summary?.prs?.[String(entry.number)] : entry.kind === "issue" && entry.taskKey ? Object.entries(summary?.issues ?? {}).find(([key]) => sameIssue(key, entry.taskKey!))?.[1] : undefined;
  return [t("links.sessions"), count ? String(count) : null];
}

export function detailOf(rest: SnapshotRest | null, entry: SearchEntry, now: number, t: TFunction<"translation">, language: InterfaceLanguage, links: LinkSummaries | null = null): Detail {
  const relations = relationsFor(rest, entry, t);
  const device = entry.chip?.label ?? searchDevices(rest ?? ({} as SnapshotRest)).find((scope) => scope.device.id === entry.deviceId)?.name;
  const base = { relations, tags: [] as string[], pills: [] as EntryStatus[] };
  switch (entry.kind) {
    case "session":
      return { ...base, kind: t("sessionWork.sessions"), title: entry.title, facts: present([[t("sessionWork.requests"), entry.subtitle], [t("search.fact.checkout"), entry.place]]), action: t("sessionWork.record") };
    case "agent": {
      return {
        ...base,
        kind: provider(entry.agentKind, t),
        title: entry.title,
        pills: entry.status ? [entry.status] : [],
        tags: device ? [device] : [],
        facts: present([[t("search.fact.checkout"), entry.place], [t("search.fact.pr"), (entry.agent?.request?.pull_requests ?? []).map((pr) => pullLabel(pr)).join(", ")], [t("search.fact.lastWords"), lastWords(entry)]]),
        action: t("search.goToAgent"),
      };
    }
    case "checkout": {
      // A Recent row of a device that is not connected: the names the record kept and nothing else, and ↵ does nothing, so no action is offered.
      if (entry.dimmed) return { ...base, kind: t("search.kind.checkout"), title: `${entry.subtitle} › ${entry.title}`, tags: [entry.title, device].filter((text): text is string => Boolean(text)), facts: [], action: "" };
      const checkout = entry.checkout;
      const files = checkout?.changed_file_count ?? 0;
      const ahead = checkout?.ahead ?? 0;
      return {
        ...base,
        kind: t("search.kind.checkout"),
        title: `${entry.workspace?.label ?? ""} › ${checkout?.label ?? entry.title}`,
        tags: [checkout?.branch, device].filter((text): text is string => Boolean(text)),
        facts: present([
          [t("search.fact.changes"), [ahead > 0 ? t("search.commitsAfterBase", { count: ahead }) : null, files > 0 ? t("search.changedFiles", { count: files }) : null].filter(Boolean).join(" · ")],
          [t("search.fact.pr"), checkout?.pull_request ? pullLabel(checkout.pull_request) : null],
          [t("issue.label"), checkout?.task_key ? (entry.workspace?.tasks?.tasks.find((task) => task.key === checkout.task_key)?.id ?? null) : null],
        ]),
        action: t("search.openCheckout"),
      };
    }
    case "pr": {
      if (entry.external) {
        return { ...base, kind: t("search.kind.githubPr"), title: entry.title.replace(/^#\d+ /, ""), pills: entry.status ? [entry.status] : [], tags: [`#${entry.number}`, entry.repository ?? ""].filter(Boolean), facts: [], action: t("issue.openGitHub") };
      }
      const pr = entry.pr;
      const closes = (pr?.closing_issues ?? []).map((issue) => `#${issue.number}`).join(", ");
      const review = reviewWord(pr?.review);
      return {
        ...base,
        kind: t("search.kind.pullRequest"),
        title: pr?.title ?? entry.title,
        pills: entry.status ? [entry.status] : [],
        tags: [pr ? pullLabel(pr) : `#${entry.number}`],
        facts: present([
          [t("search.fact.review"), review ? t(review.key) : null],
          [t("workspace.branch"), pr?.head_branch],
          [t("search.fact.closingIssues"), closes],
          sessionsFact(links, entry, t),
          [t("search.fact.read"), entry.workspace ? lastReadWords(entry.workspace, now, t, language) : null],
        ]),
        action: t("search.openPrs"),
      };
    }
    case "issue": {
      if (entry.external) {
        return { ...base, kind: t("search.kind.githubIssue"), title: entry.title.replace(/^#\d+ /, ""), pills: entry.status ? [entry.status] : [], tags: [`#${entry.number}`, entry.repository ?? ""].filter(Boolean), facts: [], action: t("issue.openGitHub") };
      }
      const owners = (entry.workspace?.checkouts ?? []).filter((checkout) => checkout.task_key === entry.taskKey);
      const closing = (entry.workspace?.checkouts ?? []).filter((checkout) => checkout.pull_request && (checkout.closes_task_keys ?? []).includes(entry.taskKey ?? ""));
      return {
        ...base,
        kind: t("issue.label"),
        title: entry.task?.title ?? entry.title,
        pills: entry.status ? [entry.status] : [],
        tags: entry.task?.id ? [entry.task.id] : [],
        facts: present([
          [t("common.project"), entry.workspace?.label],
          [t("search.fact.assigned"), owners.map((checkout) => checkout.branch ?? checkout.label).join(", ")],
          [t("search.fact.closingPrs"), closing.map((checkout) => `#${checkout.pull_request?.number}`).join(", ")],
          sessionsFact(links, entry, t),
          [t("search.fact.read"), entry.workspace ? lastReadWords(entry.workspace, now, t, language) : null],
        ]),
        action: t("search.openIssue"),
      };
    }
    case "project":
      return { ...base, kind: t("common.project"), title: entry.title, tags: device ? [device] : [], facts: present([[t("search.fact.path"), entry.workspace?.path]]), action: t("search.openProject") };
    case "device":
      return { ...base, kind: t("search.kind.device"), title: entry.title, facts: present([[t("search.fact.type"), entry.subtitle]]), action: t("search.goToDevice") };
    case "command":
      return { ...base, kind: t("search.kind.command"), title: entry.title, facts: [], action: entry.dimmed ? "" : entry.command === "open_url" ? t("search.openUrl") : t("search.openStartPanel") };
  }
}
