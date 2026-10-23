import { legacyWork } from "../test/legacyRequestWork";
import { emptyScope } from "../test/legacyAgentScope";
import { requestRows, requestGroups } from "../test/legacyAgentScope";
import { legacyAgentRow } from "../test/legacyAgentRow";
// The request view's rules (PRD overview-request-view): which rows it draws
// and in what order, the Requests tile, the one-line request (D-42), and the
// chips. The expected answers are the PRD's Behaviors and the operator
// requests measured for D-42, read against small fixtures.

import { describe, expect, it } from "vitest";
import { initializeInterfaceI18n } from "./i18n/instance";
import type { LensAgent } from "./overviewLens";
import { childrenSummary, fullRequest, openCandidates, pullRequestChip as drawPullRequestChip, requestLine, senderWords, resultLine, rowIssues as drawRowIssues, rowIssueChips as drawRowIssueChips, splitTail, verdictLine,  } from "../test/legacyRequestList";
import type { AgentPullRequest, AgentRequest, AgentRow, Checkout, RequestVerb, Task, Workspace } from "./snapshot";

const NOW = new Date(2026, 9, 3, 15, 0, 0).getTime();

const t = initializeInterfaceI18n("ko").getFixedT(null, "translation");
const english = initializeInterfaceI18n("en").getFixedT(null, "translation");

const PROJECT = { agent_scope: emptyScope(),
  id: "project",
  label: "Project",
  path: "/fixture",
  device_id: "local",
  checkouts: [],
  tasks: { source: null, tasks: [task(7), task(8)], overflow: false },
} as unknown as Workspace;

const CHECKOUT = { id: "main", label: "main", branch: "main", tabs: [] } as unknown as Checkout;

function task(number: number): Task {
  return { key: `github:acme/project#${number}`, source: "github", id: `#${number}`, url: `https://github.com/acme/project/issues/${number}`, title: `Task ${number}`, open: true };
}

function block(verb: RequestVerb, extra: Partial<AgentRequest> = {}): AgentRequest {
  return { verb, verb_since_unix_ms: NOW, request: null, later_by: null, pull_requests: [], ...extra };
}

function agent(pane: string, verb: RequestVerb | null, extra: Partial<AgentRow> = {}): AgentRow {
  return legacyAgentRow({
    id: pane,
    pane_id: pane,
    identity_label: pane,
    agent_kind: "claude",
    mark: "going",
    group: verb === "working" ? "working" : "idle",
    status_code: "unknown",
    changed_at_unix_ms: NOW,
    emphasized: false,
    unread: false,
    last_activity: "0000000000001",
    ...(verb ? { request: block(verb) } : {}),
    ...extra,
  });
}

function lens(row: AgentRow, task: Task | null = null): LensAgent {
  return { agent: row, bucket: "resting", project: PROJECT, checkout: CHECKOUT, device: null, task };
}

function pull(number: number, extra: Partial<AgentPullRequest> = {}): AgentPullRequest {
  return { number, title: `PR ${number}`, url: `https://github.com/acme/project/pull/${number}`, badge: "open", checks: "passing", head_branch: `b${number}`, closing_issues: [], live: true, duty: true, created: true, settled_at_unix_ms: null, ...extra };
}

describe("the request line (D-42, B52)", () => {
  it("joins lines with a middle dot, drops blank lines and runs of spaces, and ends with the image count", () => {
    expect(requestLine("고쳐 줘\n\n  그리고   테스트도 [Image #1]\n[Image #2]", 2, t)).toBe("고쳐 줘 · 그리고 테스트도 · 이미지 2");
  });

  it("shows a request of images alone as the image mark alone", () => {
    expect(requestLine("[Image #1]", 1, t)).toBe("이미지 1");
  });

  it("names paths and addresses by their last name, so a home folder never shows", () => {
    expect(requestLine("/Users/example/projects/app/docs/README.md 읽고 https://github.com/acme/project/pull/336 리뷰", 0, t)).toBe("README.md 읽고 #336 리뷰");
    expect(requestLine("~/work/notes/today.txt 봐", 0, t)).toBe("today.txt 봐");
  });

  it("keeps a long name's first twelve characters and its extension", () => {
    expect(requestLine("src/components/request-row-expanded-detail.tsx", 0, t)).toBe("request-row-….tsx");
  });

  it("drops characters that would reverse or hide part of a chip's name", () => {
    const labels = openCandidates("받은 파일 https://example.com/x/report%E2%80%AEfdp.exe 확인", [], false).map((candidate) => candidate.label);
    expect(labels).toEqual(["reportfdp.exe"]);
  });

  it("names an address whose last part is not valid percent-encoding as written", () => {
    expect(requestLine("세일 https://example.com/files/sale-50% 확인", 0, t)).toBe("세일 sale-50% 확인");
    expect(openCandidates("보고서 https://example.com/x/%zz 를 보세요", [], false).map((target) => target.label)).toEqual(["%zz"]);
  });

  it("keeps a slash between two words of a sentence", () => {
    expect(requestLine("PR/이슈 둘 다 봐줘", 0, t)).toBe("PR/이슈 둘 다 봐줘");
  });
});

describe("fitting the request line", () => {
  // One unit per character stands in for the font.
  const width = 40;
  const fits = (text: string) => [...text].length <= width;
  const fitsTail = (text: string) => [...text].length <= width * 0.4;

  it("leaves a line that fits whole", () => {
    expect(splitTail("짧은 요청", fits, fitsTail)).toEqual({ head: "짧은 요청", tail: "" });
  });

  it("keeps the end's whole words within forty percent and leaves the front to be cut", () => {
    const line = "Overview 요청 보기의 긴 요청 줄을 앞과 끝으로 나눠서 보여 주고 끝쪽 단어는 남겨 둘 것";
    const { head, tail } = splitTail(line, fits, fitsTail);
    // Sixteen characters, forty percent of forty.
    expect(tail).toBe("주고 끝쪽 단어는 남겨 둘 것");
    expect(`${head} ${tail}`).toBe(line);
  });

  it("cuts only the front when the last word alone is too wide", () => {
    const line = `앞 ${"x".repeat(30)} ${"y".repeat(20)}`;
    expect(splitTail(line, fits, fitsTail)).toEqual({ head: line, tail: "" });
  });

  it("shows twenty lines of the full request, then offers the rest", () => {
    const text = Array.from({ length: 25 }, (_, index) => `줄 ${index + 1}`).join("\n");
    expect(fullRequest(text, false)).toEqual({ text: text.split("\n").slice(0, 20).join("\n"), more: true });
    expect(fullRequest(text, true)).toEqual({ text, more: false });
  });
});

describe("the rows and groups (D-06, D-30, D-40)", () => {
  it("draws the groups in the PRD's order, leaves out empty ones, and puts the longest wait first in a to-do group", () => {
    const rows = requestRows(
      [
        lens(agent("idle", "idle")),
        lens(agent("late", "answer", { request: block("answer", { verb_since_unix_ms: NOW - 1_000 }) })),
        lens(agent("early", "answer", { request: block("answer", { verb_since_unix_ms: NOW - 60_000 }) })),
        lens(agent("fix", "fix")),
        lens(agent("old-work", "working", { last_activity: "0000000000001" })),
        lens(agent("new-work", "working", { last_activity: "0000000000009" })),
      ],
      [],
    );
    const groups = requestGroups(rows);
    expect(groups.map((group) => group.verb)).toEqual(["answer", "fix", "working", "idle"]);
    expect(groups[0]!.rows.map((row) => row.lens.agent.pane_id)).toEqual(["early", "late"]);
    expect(groups[2]!.rows.map((row) => row.lens.agent.pane_id)).toEqual(["new-work", "old-work"]);
  });

  it("folds a delegated child into its parent's row and keeps an orphaned child as its own", () => {
    const parent = agent("parent", "waiting", { close_descendant_pane_ids: ["grandchild", "child"], descendant_counts: { error: 0, approval: 1, question: 1, working: 1, done: 0 } });
    const child = agent("child", "working", { delegated: true, lineage_parent_pane_id: "parent" });
    const grandchild = agent("grandchild", "working", { delegated: true, lineage_parent_pane_id: "child" });
    const orphan = agent("orphan", "idle", { delegated: true, lineage_parent_pane_id: "gone" });
    const rows = requestRows([lens(parent), lens(child), lens(grandchild), lens(orphan)], [parent, child, grandchild, orphan]);
    expect(rows.map((row) => row.lens.agent.pane_id)).toEqual(["parent", "orphan"]);
    expect(rows[0]!.children.map((row) => row.pane_id)).toEqual(["child", "grandchild"]);
    expect(childrenSummary(rows[0]!, t)).toEqual({ text: "자식 2 · 작업 중 1", asking: 2 });
    expect(childrenSummary(rows[1]!, t)).toBeNull();
  });

  it("reads a row the core has not laid a block on by its group", () => {
    const rows = requestRows([lens(agent("w", null, { group: "working" })), lens(agent("r", null))], []);
    expect(rows.map((row) => row.verb)).toEqual(["working", "idle"]);
  });
});

describe("the result line (D-12, B5, B10)", () => {
  it("is the label's line when the agent has one, and nothing without it", () => {
    const [row, quiet] = requestRows([lens(agent("a", "result", { request: block("result", { line: "테스트 통과" }) })), lens(agent("q", "result", { request: block("result") }))], []);
    expect(resultLine(row!)).toBe("테스트 통과");
    expect(resultLine(quiet!)).toBe("");
  });

  it("reads the label's verdict in the expanded row, and nothing without a label (B6)", () => {
    expect(verdictLine(block("answer", { end: "question", line: "어느 쪽으로 할까요?" }), t)).toBe("AI 판정 · 질문 · 어느 쪽으로 할까요?");
    expect(verdictLine(block("stopped", { end: "unfinished" }), t)).toBe("AI 판정 · 일 남음");
    expect(verdictLine(block("result"), t)).toBeNull();
    expect(verdictLine(undefined, t)).toBeNull();
  });
});

describe("the pull request and issue chips (D-46, D-47)", () => {
  it("shows the core's first live pull request with the other live ones as +N", () => {
    expect(pullRequestChip([pull(1, { checks: "failed" }), pull(2), pull(3, { live: false, badge: "merged" })])).toEqual({ chip: pull(1, { checks: "failed" }), more: 1 });
    expect(pullRequestChip([pull(3, { live: false, badge: "merged" })])).toBeNull();
  });

  it("orders the issues the chip closes, then the checkout's, then the others', each once", () => {
    const pulls = [pull(1, { closing_issues: [{ repository: "acme/project", number: 8 }] }), pull(2, { closing_issues: [{ repository: "acme/project", number: 7 }, { repository: "acme/project", number: 99 }] })];
    const [row] = requestRows([lens(agent("a", "review", { request: block("review", { pull_requests: pulls }) }), task(7))], []);
    expect(rowIssues(row!, PROJECT).map((issue) => issue.id)).toEqual(["#8", "#7"]);
  });
  it("keeps old closed issues in expanded history but only current closed and open issues in folded chips", () => {
    const issues = [
      { ...task(7), open: false, closed_at_unix_ms: NOW - 1, updated_at_unix_ms: NOW + 100 },
      { ...task(8), open: false, closed_at_unix_ms: NOW + 1 },
      task(9),
      { ...task(10), open: false },
      { ...task(11), open: false, closed_at_unix_ms: NOW },
    ];
    const project = { ...PROJECT, tasks: { ...PROJECT.tasks!, tasks: issues } };
    const pulls = [
      pull(42, { live: false, badge: "merged", settled_at_unix_ms: NOW - 1, closing_issues: [{ repository: "acme/project", number: 7 }] }),
      pull(43, { badge: "merged", settled_at_unix_ms: NOW + 1, closing_issues: [{ repository: "acme/project", number: 8 }] }),
      pull(44, { closing_issues: [9, 10, 11].map((number) => ({ repository: "acme/project", number })) }),
    ];
    const request = { text: "Next work", cut: false, images: 0, at_unix_ms: NOW, sender: { kind: "operator" as const } };
    const [row] = requestRows([lens(agent("a", "result", { request: block("result", { request, pull_requests: pulls }) }))], []);
    expect(rowIssueChips(row!, project).map((issue) => issue.id)).toEqual(["#8", "#9"]);
    expect(rowIssues(row!, project).map((issue) => issue.id)).toEqual(["#8", "#7", "#9", "#10", "#11"]);
    const checkoutOnly = { ...row!, lens: { ...row!.lens, task: issues[0]! } };
    checkoutOnly.lens.agent = { ...checkoutOnly.lens.agent, request: block("idle", { request }) };
    expect(rowIssueChips(checkoutOnly, project)).toEqual([]);
    expect(rowIssues(checkoutOnly, project)).toEqual([issues[0]]);
    expect(rowIssueChips({ ...checkoutOnly, lens: { ...checkoutOnly.lens, task: { ...issues[1]!, source: "local", id: "L-8" } } }, project).map((issue) => issue.id)).toEqual(["L-8"]);
  });
});

describe("open targets (D-39, B49)", () => {
  const reply = "PR https://github.com/acme/project/pull/9 와 `docs/README.md`, https://example.com/report 를 보세요. 다시 https://example.com/report";

  it("keeps each URL and path once, in order, without the row's own pull request", () => {
    const found = openCandidates(reply, ["https://github.com/acme/project/pull/9"], true);
    expect(found.map((target) => [target.label, target.target.kind])).toEqual([
      ["README.md", "path"],
      ["report", "url"],
    ]);
  });

  it("offers a device's row its URLs only", () => {
    expect(openCandidates(reply, [], false).map((target) => target.label)).toEqual(["#9", "report"]);
  });

  it("extracts Markdown destinations before validation, with spaced labels and balanced paths", () => {
    const markdown = '[Read report](https://example.com/report) [report](/repo/report.md) [local notes](<docs/local notes.md>) [nested](https://example.com/a_(b)) [same](https://example.com/report) [PR](https://github.com/acme/project/pull/9) [unsafe](javascript:alert(1))';
    expect(openCandidates(markdown, ["https://github.com/acme/project/pull/9"], true).map((candidate) => candidate.key)).toEqual([
      "https://example.com/report", "/repo/report.md", "docs/local notes.md", "https://example.com/a_(b)",
    ]);
    expect(openCandidates(markdown, [], false).every((candidate) => candidate.target.kind === "url")).toBe(true);
  });
});

describe("the words of a row follow the interface language", () => {
  it("writes the image count, the sender and the verdict in English", () => {
    expect(requestLine("fix it [Image #1]", 1, english)).toBe("fix it · 1 image");
    expect(requestLine("[Image #1] [Image #2]", 2, english)).toBe("2 images");
    expect(senderWords({ kind: "operator" }, english)).toBe("Me");
    expect(senderWords({ kind: "named", name: "Planner" }, english)).toBe("Planner");
    expect(senderWords({ kind: "agent" }, english)).toBe("Agent");
    expect(verdictLine(block("answer", { end: "question", line: "Which one?" }), english)).toBe("AI assessment · Question · Which one?");
    expect(verdictLine(block("stopped", { end: "unfinished" }), english)).toBe("AI assessment · Unfinished");
  });

  it("counts descendants with the language's plural and joins the working count", () => {
    const parent = agent("parent", "waiting", { close_descendant_pane_ids: ["child"], descendant_counts: { error: 0, approval: 0, question: 0, working: 1, done: 0 } });
    const child = agent("child", "working", { delegated: true, lineage_parent_pane_id: "parent" });
    const [row] = requestRows([lens(parent), lens(child)], [parent, child]);
    expect(childrenSummary(row!, english)).toEqual({ text: "1 descendant · Working 1", asking: 0 });
  });

});

function withWork(row: import("../test/legacyRequestList").RequestRow, project: Workspace) {
  return { ...project, agent_scope: { ...emptyScope(), work: { [row.lens.agent.pane_id]: legacyWork(row.lens, project) } } };
}
function rowIssues(row: import("../test/legacyRequestList").RequestRow, project: Workspace) { return drawRowIssues(row, withWork(row, project)); }
function rowIssueChips(row: import("../test/legacyRequestList").RequestRow, project: Workspace) { return drawRowIssueChips(row, withWork(row, project)); }
function pullRequestChip(pulls: AgentPullRequest[]) {
  const [row] = requestRows([lens(agent("a", "review", { request: block("review", { pull_requests: pulls }) }))], []);
  return drawPullRequestChip({ ...row!, lens: { ...row!.lens, project: withWork(row!, PROJECT) } });
}
