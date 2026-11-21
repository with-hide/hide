// A pull request card as the lane draws it: its checks, the sessions that made it and the merge button, drawn
// by the product's own `PrCardView` over one invented pull request. Nothing leaves the page: every action goes
// to a dispatcher that accepts and ignores it.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../../actions";
import { PrCardView } from "../../PrCard";
import type { LinkedSession, PullRequest } from "../../snapshot";
import { useShellStore } from "../../store";
import { useUiStore } from "../../ui";
import { minutesAgo } from "../fixtures/agent";
import { docsProject } from "../fixtures/project";
import { PartFrame } from "../frame";

const NUMBER = 412;

function fixture() {
  const { rest, project, checkout } = docsProject();
  const pr: PullRequest = {
    number: NUMBER, title: "Move the orders API to v2", url: "https://example.invalid/pull/412", badge: "open", state: "mergeable", is_draft: false, review: "approved", checks: "passing",
    head_branch: "feat/orders-v2", base_branch: "main", closing_issues: [], created_at_unix_ms: minutesAgo(70), updated_at_unix_ms: minutesAgo(4),
    work: {
      author: "you", author_is_bot: false, mine: true, merge_state: "clean", review_requests: [], asked_of_me: "none", failed_checks: [], pending_checks: 0,
      check_counts: { total: 12, passed: 12, failed: 0, running: 0, waiting: 0, skipped: 0, unknown: 0 }, queue: { state: "not_queued" }, review_requested_at_unix_ms: null,
    },
  };
  checkout.branch = pr.head_branch!;
  checkout.pull_request = pr;
  checkout.github = { available: true, loading: false, stale: false, failure_category: null, last_success_at_unix_ms: minutesAgo(1) };
  project.pull_requests = [pr];
  project.agent_scope!.prs = { rows: [{ number: NUMBER, group: "needs_you", fold: null, reason: { kind: "merge" }, agents: [], agent_pane: null, checkout_id: checkout.id, checkout_ids: [checkout.id], issue: null }], groups: [{ group: "needs_you", numbers: [NUMBER] }], needs_you: 1 };
  rest.pr_work = {
    link: null, feedback: null,
    cards: [{
      workspace_id: project.id, number: NUMBER, reading: false, head: "a".repeat(40),
      body: "## Summary\nThe orders API now lives under `/v2`. Every caller moved to it, and the old endpoint answers for one more release.\n\n## Checks\nAll twelve checks pass.",
      files: [{ path: "api/orders/v2.ts", additions: 120, deletions: 18, generated: false }, { path: "api/orders/callers.ts", additions: 36, deletions: 31, generated: false }],
      files_total: 2, threads: [], merge: { can_merge: true, methods: ["merge"], queue: false }, failed: [], write: null,
    }],
  };
  const session = (id: string, agent: string, started: number, ended: number | null, role: LinkedSession["role"], request: string): LinkedSession => ({
    agent, id, ids: [id], device_id: "local", role, pr: NUMBER, request, started_at_unix_ms: started, ended_at_unix_ms: ended, path: `/sessions/${id}`, cwd: checkout.path, file: "present", parent: null,
  });
  const sessions = [
    session("docs-made", "claude", minutesAgo(70), minutesAgo(30), "created", "Move the orders API to v2"),
    session("docs-reviewed", "codex", minutesAgo(25), minutesAgo(10), "reviewed", "Review the v2 change"),
  ];
  return { rest, project, pr, sessions };
}

export default function PrCardPart() {
  const { rest, project, pr, sessions } = useMemo(fixture, []);
  const actions = useMemo(() => createActions(() => true), []);
  useLayoutEffect(() => {
    useShellStore.setState({
      rest, agents: [], connection: "live",
      linkPanel: {
        workspace_id: project.id, target: { kind: "pr", number: NUMBER }, loading: false, failure: null,
        pr: { number: NUMBER, branch: pr.head_branch!, title: pr.title, url: pr.url, created_at_unix_ms: pr.created_at_unix_ms!, closed_at_unix_ms: null, merged_at_unix_ms: null, state: pr.state, issues: [], worktrees: [] },
        prs: [], sessions, total: sessions.length,
      },
    });
    useUiStore.setState({ screen: { kind: "workspace" } });
  }, [rest, project, pr, sessions]);
  return (
    <PartFrame part="pr-card">
      <div className="flex min-w-0 flex-col rounded-md border border-border"><PrCardView workspaceId={project.id} number={NUMBER} actions={actions} /></div>
    </PartFrame>
  );
}
