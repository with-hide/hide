import { describe, expect, it } from "vitest";
import { memoryCards, memoryTime, sourceTime, visibleCards, memoryProgress, memoryProgressAgent, memoryProgressStatus } from "./memoryList";
import type { AgentPullRequest, AgentRow, MemoryRow, ProjectMemory, PullRequest } from "./snapshot";
import { legacyAgentRow } from "../test/legacyAgentRow";

const row = (number: number): MemoryRow => ({
  number, title: `card ${number}`, kind: "rule", human_source_count: 0, provided_session_count: 0,
  pull_count: 0, updated_at_unix_ms: 0, verifying: false, promotion: null,
});

describe("Memory promotion rows", () => {
  const promotion = { state: "pr_open" as const, agent: null, pr_number: 72, pr_url: "https://github.com/acme/one/pull/72", target_path: null, github_failure: null, started_at_unix_ms: 1 };
  const grouped = () => memoryProgress(project({ rows: [{ ...row(9), promotion }, { ...row(7), promotion }] }))[0]!;
  const pull: PullRequest = { number: 72, title: "Document the rule", url: "https://github.com/acme/one/pull/72", badge: "review", review: "review_required", is_draft: false, state: "pending", checks: "passing" };
  const agentPull: AgentPullRequest = { ...pull, checks: "passing", head_branch: "memory-rule", closing_issues: [], live: true, duty: true, created: false, settled_at_unix_ms: null };
  const agent = (origin = true, linked = agentPull): AgentRow => legacyAgentRow({
    id: "a", pane_id: "w1:p2", identity_label: "memory writer", agent_kind: "claude", group: "working", status_code: "working", changed_at_unix_ms: 1, emphasized: false, unread: false,
    hide_ai: origin ? { task: { kind: "memory_promote", target: "m7" } } : undefined,
    request: { verb: "working", verb_since_unix_ms: 1, request: null, later_by: null, reply: null, pull_requests: [linked] },
  });

  it("groups confirmed cards in number order without duplicating the expanded row in Cards", () => {
    expect(grouped().cards.map((card) => card.number)).toEqual([7, 9]);
    const active = project({ rows: grouped().cards, expanded: { number: 7, body: "text", cited_paths: [], sources: [], previous: null } });
    expect(memoryCards(active)).toEqual([]);
    const running = (number: number, start: number) => ({ ...row(number), promotion: { ...promotion, state: "running" as const, pr_number: null, started_at_unix_ms: start, agent: { name: "writer", pane_id: "w1:p2" } } });
    expect(memoryProgress(project({ rows: [running(1, 2), running(2, 2), running(3, 3)] })).map((group) => group.cards.map((card) => card.number))).toEqual([[1, 2], [3]]);
  });

  it("requires a promotion origin and its live PR in this repository, even when numbers or names match", () => {
    const unrelatedRepo = agent(true, { ...agentPull, url: "https://github.com/acme/two/pull/72" });
    const ended = agent(true, { ...agentPull, live: false });
    const unrelatedTask = { ...agent(), hide_ai: { task: { kind: "inspect", target: "m7" } } };
    expect(memoryProgressAgent(grouped(), pull, [agent(false), unrelatedRepo, ended, unrelatedTask])).toBeUndefined();
    expect(memoryProgressAgent(grouped(), undefined, [agent()])).toBeUndefined();
    const writer = agent();
    expect(memoryProgressAgent(grouped(), pull, [unrelatedRepo, writer])).toBe(writer);
    // A batch's first card can be skipped while its successful peers share this PR.
    const skippedStarter = { ...writer, hide_ai: { task: { kind: "memory_promote", target: "m1" } } };
    expect(memoryProgressAgent(grouped(), pull, [skippedStarter])).toBe(skippedStarter);
  });

  it("groups by original repository identity and keeps unknown legacy PRs separate", () => {
    const cards = [
      { ...row(1), promotion },
      { ...row(2), promotion: { ...promotion, pr_url: "https://github.com/acme/two/pull/72" } },
      { ...row(3), promotion: { ...promotion, pr_url: null } },
      { ...row(4), promotion: { ...promotion, pr_url: null } },
    ];
    expect(memoryProgress(project({ rows: cards })).map((group) => group.cards.map((card) => card.number))).toEqual([[1], [2], [3], [4]]);
    const other = { ...pull, url: "https://github.com/acme/two/pull/72" };
    expect(memoryProgressAgent(grouped(), other, [agent(true, { ...agentPull, url: other.url })])).toBeUndefined();
    const mixed = { ...row(5), promotion: { ...promotion, pr_url: "https://github.com/Acme/One/pull/72" } };
    expect(memoryProgress(project({ rows: [cards[0]!, mixed] })).map((group) => group.cards.map((card) => card.number))).toEqual([[1, 5]]);
    expect(memoryProgressAgent(grouped(), { ...pull, url: "https://github.com/Acme/One/pull/72" }, [agent()])).toBeDefined();
  });

  it("shows only observed problems and leaves quiet review neutral without a conversation link", () => {
    expect(memoryProgressStatus(grouped(), pull, agent())).toMatchObject({ key: "memory.prOpen", tone: "text-muted-foreground", agentLink: false });
    expect(memoryProgressStatus(grouped(), { ...pull, checks: "failed" }, agent())).toMatchObject({ key: "agentSessions.pr.failed", tone: "text-destructive", agentLink: true });
    expect(memoryProgressStatus(grouped(), { ...pull, review: "changes_requested" }, agent())).toMatchObject({ key: "board.review.changes", tone: "text-warning" });
    const asking = agent();
    asking.state.asking = true;
    expect(memoryProgressStatus(grouped(), pull, asking)).toMatchObject({ key: "memory.progress.asking", tone: "text-warning", agentLink: true });
    expect(memoryProgressStatus(grouped(), undefined, undefined).key).toBe("memory.progress.inPr");
    expect(memoryProgressStatus(grouped(), { ...pull, state: "merged", checks: "failed" }, asking)).toMatchObject({ key: "agentSessions.pr.merged", agentLink: false });
  });
});
const project = (overrides: Partial<ProjectMemory>): ProjectMemory => ({
  workspace_id: "p", unavailable: null, loading: false, enabled: true, capacity_reached: false, evicted_cards: 0,
  rows: [1, 2, 3, 4, 5, 6, 7].map(row), questions: [], analysis: "idle", consent_required: false, usable_cards: 2, checking_card: null, check_resume_at_unix_ms: null, forgotten: null,
  pending_action: null, expanded: null, search: null, ...overrides,
});

describe("the Memory tab's card list", () => {
  it("draws a question's card once, in 물어볼 것, and a search's answer in its own order", () => {
    const asked = project({ questions: [{ kind: "failed", number: 2, title: "card 2", start_failure: null }] });
    expect(memoryCards(asked).map((card) => card.number)).toEqual([1, 3, 4, 5, 6, 7]);
    const searched = project({ search: { query: "x", numbers: [6, 2, 9] } });
    expect(memoryCards(searched).map((card) => card.number)).toEqual([6, 2]);
  });

  it("folds the cards past the first five, but never the card shown in full", () => {
    const cards = project({}).rows;
    expect(visibleCards(cards, null, false)).toEqual({ shown: cards.slice(0, 5), folded: 2 });
    const opened = visibleCards(cards, 7, false);
    expect(opened.shown.map((card) => card.number)).toEqual([1, 2, 3, 4, 5, 7]);
    expect(opened.folded).toBe(1);
    expect(visibleCards(cards, null, true).folded).toBe(0);
    // A card on its way into the repository is never folded either.
    const moving = cards.map((card) => (card.number === 6 ? { ...card, promotion: { state: "running" as const, agent: null, pr_number: null, target_path: null, github_failure: null } } : card));
    expect(visibleCards(moving, null, false).shown.map((card) => card.number)).toEqual([1, 2, 3, 4, 5, 6]);
  });

  it("names a row's time as minutes, hours, yesterday, then the date", () => {
    const now = new Date(2026, 9, 12, 15, 0).getTime();
    expect(memoryTime(now - 12 * 60_000, "ko", now)).toBe("12분 전");
    expect(memoryTime(now - 3 * 3_600_000, "ko", now)).toBe("3시간 전");
    expect(memoryTime(new Date(2026, 9, 11, 9, 0).getTime(), "ko", now)).toBe("어제");
    expect(memoryTime(new Date(2026, 9, 7, 9, 0).getTime(), "ko", now)).toBe("10-07");
    expect(memoryTime(new Date(2025, 0, 2, 9, 0).getTime(), "ko", now)).toBe("2025-01-02");
  });

  it("names a source's time with its clock, as the design's 어제 14:02 and 10-09 21:40", () => {
    const now = new Date(2026, 9, 12, 15, 0).getTime();
    expect(sourceTime(new Date(2026, 9, 12, 9, 5).getTime(), "ko", now)).toBe("09:05");
    expect(sourceTime(new Date(2026, 9, 11, 14, 2).getTime(), "ko", now)).toBe("어제 14:02");
    expect(sourceTime(new Date(2026, 9, 9, 21, 40).getTime(), "ko", now)).toBe("10-09 21:40");
  });
});
