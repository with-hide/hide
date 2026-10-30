// The real PR list over invented pull requests in each review state (PRD
// pr-review-status B1-B4, B9, B11, B15). No GitHub read or write leaves it.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { PrList } from "../PrList";
import { TooltipProvider } from "../components/ui/tooltip";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import { requireInterfaceLanguage } from "../i18n/locale";
import { prListModel } from "../prWork";
import type { PullRequest } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import { AT, applyReview, reviewPlace } from "./PrCardScene";
import { REFERENCE_FOLDS, sidebarScene } from "./sceneData";

/** Each row's review state, and the one whose reviewers could not be read. */
const ROWS: { number: number; title: string; review: string | null; unread?: boolean; draft?: boolean }[] = [
  { number: 1401, title: "feat: show who approved a pull request", review: "partial" },
  { number: 1402, title: "fix: keep the merge queue place after a rename", review: "no-reviewer" },
  { number: 1403, title: "docs: say where the release notes live", review: "approved" },
  { number: 1404, title: "feat: 리뷰어 이름이 길어도 행이 넘치지 않게 하기 - a very long title in two scripts", review: "unknown-m" },
  { number: 1405, title: "chore: bump the pinned toolchain", review: "not-required" },
  { number: 1406, title: "refactor: split the link worker's mailbox", review: "partial", unread: true },
  { number: 1407, title: "wip: try a second layout", review: null, draft: true },
];

export function PrListScene({ params }: { params: URLSearchParams }) {
  useFollowInterfaceLanguage(requireInterfaceLanguage(params.get("lang") ?? "ko"));
  const theme = params.get("theme") === "light" ? "light" : "dark";
  const fixture = useMemo(() => {
    const base = sidebarScene("reference", REFERENCE_FOLDS, AT);
    const project = base.rest.navigator!.workspaces!.find((row) => row.id === "hide")!;
    const checkout = project.checkouts[0]!;
    checkout.github = { available: true, loading: false, stale: false, failure_category: null, last_success_at_unix_ms: AT, unread_queries: params.get("unread") === "reviews" ? ["merge_queue", "reviews"] : [] };
    const pulls: PullRequest[] = ROWS.map((row) => {
      const pr: PullRequest = {
        number: row.number, title: row.title, url: `https://github.com/example/hide/pull/${row.number}`, badge: "open", state: "pending", is_draft: row.draft === true, review: null, checks: "passing", head_branch: `feat/${row.number}`, base_branch: "main", closing_issues: [], created_at_unix_ms: AT - 86_400_000, updated_at_unix_ms: AT - 1_800_000,
        work: { author: "maintainer", author_is_bot: false, mine: true, merge_state: "clean", review_requests: [], asked_of_me: "none", failed_checks: [], pending_checks: 0, queue: { state: "not_queued" }, review_requested_at_unix_ms: null },
      };
      if (row.review) applyReview(pr, row.review);
      if (row.draft) pr.state = "draft";
      if (row.unread || params.get("unread") === "reviews") { pr.work!.reviewers = null; pr.work!.required_approvals = null; }
      return pr;
    });
    project.pull_requests = pulls;
    const rows = ROWS.map((row) => {
      const placed = row.draft ? { group: "drafts" as const, reason: { kind: "draft" as const, since_unix_ms: AT - 86_400_000 } } : reviewPlace(row.review ?? "not-required");
      return { number: row.number, group: placed.group, fold: null, reason: placed.reason, agents: [], agent_pane: null, checkout_id: checkout.id, checkout_ids: [checkout.id], issue: null };
    });
    const groups = (["needs_you", "waiting", "drafts"] as const).map((group) => ({ group, numbers: rows.filter((row) => row.group === group).map((row) => row.number) })).filter((group) => group.numbers.length > 0);
    project.agent_scope!.prs = { rows, groups, needs_you: rows.filter((row) => row.group === "needs_you").length };
    return { base, model: prListModel([{ workspace: project, agents: [], device: null }], null, AT) };
  }, [params]);
  const actions = useMemo(() => createActions(() => true), []);
  useLayoutEffect(() => {
    useShellStore.setState({ rest: fixture.base.rest, agents: [], connection: "live" });
    useUiStore.setState({ screen: { kind: "workspace" } });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", params.get("scale") ?? "1");
  }, [fixture, theme, params]);
  return <TooltipProvider><div style={{ width: Number(params.get("width") ?? 360) }} className="flex h-full min-w-0 flex-col bg-background text-foreground" data-gallery-scene="pr-list"><PrList model={fixture.model} actions={actions} onlyCheckout={false} showProject={false} onAllCheckouts={() => {}} /></div></TooltipProvider>;
}
