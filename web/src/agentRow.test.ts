import { sectionTree, sectionCount, directChildren, unfoldedRows } from "../test/legacyRowTree";
import { legacyAgentRow } from "../test/legacyAgentRow";
import { describe, expect, it } from "vitest";
import { badgeParts, badgeWords, branchChip, forkLabel, lineParts, lineText, markWords, rowAccessibleName, rowDimmed, lineShownAtRest, lineTone, markTone, rowLine, sidebarLine, treeLine } from "./agentRow";
import { createInterfaceI18n } from "./i18n/instance";
import type { AgentRow } from "./snapshot";

function row(pane: string, patch: Partial<AgentRow> = {}): AgentRow {
  return legacyAgentRow({
    id: pane,
    pane_id: pane,
    identity_label: pane,
    agent_kind: "claude",
    group: "working",
    status_code: "working",
    detail: null,
    changed_at_unix_ms: null,
    emphasized: false,
    unread: false,
    demand: "none",
    activity: "working",
    ...patch,
  });
}

describe("the second line (sidebar-agent-status D-05, B7, B8)", () => {
  it("keeps a request on screen in the warning colour until it is resolved, dimmed like its mark once read", () => {
    const asking = row("q", { demand: "question", activity: "stopped", detail: "PR 병합 전 검증을 다시 돌려도 될까요?", group: "seen", unread: false });
    const line = rowLine(asking)!;
    expect(line.mode).toBe("request");
    expect(lineShownAtRest(line, false)).toBe(true);
    expect(lineTone(line, asking)).toBe("text-warning opacity-(--opacity-read-status)");
    const unread: AgentRow = legacyAgentRow({ ...asking, group: "needs_you", emphasized: true, unread: true });
    expect(lineTone(line, unread)).toBe("text-warning");
    const blocked = row("e", { demand: "error", detail: "디스크 여유가 없어 검증을 못 함", group: "needs_you", emphasized: true });
    expect(lineTone(rowLine(blocked)!, blocked)).toBe("text-warning");
  });

  it("shows a changed row's sentence bright until the operator reads it, then only on selection", () => {
    const changed = row("w", { detail: "main 브랜치에 커밋하고 서버 시작", unread: true });
    expect(rowLine(changed)).toEqual({ text: "main 브랜치에 커밋하고 서버 시작", mode: "news", pull_request: undefined });
    expect(lineTone(rowLine(changed)!, changed)).toBe("text-foreground");
    const read = rowLine(legacyAgentRow({ ...changed, unread: false }))!;
    expect(read.mode).toBe("quiet");
    expect(lineShownAtRest(read, false)).toBe(false);
    expect(lineShownAtRest(read, true)).toBe(true);
  });

  it("draws no second line for a row the core gave no sentence, and never a progress number", () => {
    expect(rowLine(row("s", { group: "seen", activity: "stopped", detail: null, unread: true }))).toBeNull();
    expect(rowLine(row("blank", { detail: "   " }))).toBeNull();
  });
});

describe("the mark and the badge (status-marks D-01, D-02, B19)", () => {
  it("dims only a read turn; every other mark draws in its own colour", () => {
    expect(markTone(row("ask", { demand: "question", activity: "stopped", group: "needs_you", emphasized: true, unread: true }))).toBe("");
    expect(markTone(row("asked", { demand: "question", activity: "stopped", group: "seen" }))).toBe("opacity-(--opacity-read-status)");
    expect(markTone(row("blocked-read", { demand: "error", activity: "stopped", group: "seen" }))).toBe("opacity-(--opacity-read-status)");
    expect(markTone(row("root", { activity: "stopped", wait: "children" }))).toBe("");
    expect(markTone(row("idle", { activity: "stopped", mark: "quiet" }))).toBe("");
  });

  it("lists the badge's marks by whose move is next and leaves the zero ones out", () => {
    const counts = { turn: 1, going: 2, done: 0, unread: 3 };
    expect(badgeParts(counts).map((part) => `${part.mark}${part.count}`)).toEqual(["turn1", "going2", "unread3"]);
    expect(badgeParts({ unread: 1, done: 1, going: 1, turn: 1 }).map((part) => part.mark)).toEqual(["turn", "going", "done", "unread"]);
    expect(badgeParts(undefined)).toEqual([]);
  });

  it("reads the badge in the selected language", async () => {
    const { t } = await createInterfaceI18n("ko");
    expect(badgeWords({ turn: 2, going: 3, done: 1, unread: 0 }, t)).toBe("내 차례 2개, 진행 중 3개, 끝나서 읽을 것 1개");
  });

  it("names a mark with the word that leads its line (B3)", async () => {
    const { t } = await createInterfaceI18n("ko");
    expect(markWords(t, { mark: "turn", status_code: "question" })).toBe("내 차례 · 질문");
    expect(markWords(t, { mark: "turn", status_code: "approval" })).toBe("내 차례 · 승인");
    expect(markWords(t, { mark: "going", status_code: "waiting" })).toBe("진행 중");
    expect(markWords(t, { mark: "done", status_code: "done" })).toBe("읽을 것 · 끝남");
    expect(markWords(t, { mark: "unread", status_code: "unfinished" })).toBe("읽을 것 · 일 남음");
    expect(markWords(t, { mark: "unread", status_code: "stopped" })).toBe("읽을 것 · 멈춤");
    expect(markWords(t, { mark: "quiet", status_code: "idle" })).toBe("조용함");
    expect(markWords(t, { mark: "unknown", status_code: "unknown" })).toBe("상태 모름");
    // A finished turn names the tasks it left (B27); a sleeping row says only its own words (B31).
    expect(markWords(t, { mark: "done", status_code: "done", background_left: 2 })).toBe("읽을 것 · 끝남 · 백그라운드 2개 남음");
    expect(markWords(t, { mark: "asleep", status_code: "sleeping" })).toBe("재움 · 열면 깨어남");
    // In progress names nothing more while the agent works, and its wait while it waits (D-25, B36).
    expect(markWords(t, { mark: "going", status_code: "working" })).toBe("진행 중");
    expect(markWords(t, { mark: "waiting", status_code: "waiting" })).toBe("진행 중 · 대기");
  });
});

describe("the branch chip (B9)", () => {
  it("names a delegated row's checkout only when the core says it differs from its parent's", () => {
    expect(branchChip(row("child", { delegated: true, lineage_worktree_badge: "web-design-system-reset" }))).toBe("web-design-system-reset");
    expect(branchChip(row("same", { delegated: true, lineage_worktree_badge: null }))).toBeNull();
    expect(branchChip(row("root", { delegated: false, lineage_worktree_badge: "main" }))).toBeNull();
  });
});

describe("the row's accessible name", () => {
  it("reads out everything the row shows, since its visible text is hidden from assistive technology", async () => {
    const { t } = await createInterfaceI18n("en");
    const child = row("child", { identity_label: "웹 디자인 시스템 리셋 구현", delegated: true, lineage_worktree_badge: "web-design-system-reset", detail: "토큰 이관 중" });
    expect(rowAccessibleName(t, child, "mini")).toBe("웹 디자인 시스템 리셋 구현, mini, web-design-system-reset, Claude Code, In progress, 토큰 이관 중");
  });
});

// PRD fork-operator-root B8, B9, B22: what a fork's mark and row name say.
describe("a fork's mark", () => {
  it("names the source while it runs the forked conversation, and only that it was forked once it does not", async () => {
    const en = (await createInterfaceI18n("en")).t;
    const ko = (await createInterfaceI18n("ko")).t;
    const named = row("fork", { forked_from: { pane_id: "source", label: "Agent two" } });
    expect(forkLabel(en, named)).toBe("Forked from Agent two");
    expect(forkLabel(ko, named)).toBe("Agent two에서 포크됨");
    expect(forkLabel(en, row("fork", { forked_from: {} }))).toBe("Forked from another pane");
    expect(forkLabel(en, row("child", { delegated: true }))).toBeNull();
  });
});

describe("the tree a group section draws (D-03, B6)", () => {
  const parent = row("p", { activity: "stopped", wait: "children", lineage_child_pane_ids: ["c1", "c2"], lineage_collapsed: true });
  const c1 = row("c1", { delegated: true, lineage_depth: 1, lineage_child_pane_ids: ["g"], lineage_collapsed: true });
  const c2 = row("c2", { delegated: true, lineage_depth: 1, demand: "question" });
  const g = row("g", { delegated: true, lineage_depth: 2 });
  const all = new Map([parent, c1, c2, g].map((agent) => [agent.pane_id, agent]));
  const lookup = (_device: string | null, id: string) => all.get(id);
  const none = () => 0;

  it("hides a folded parent's descendants and never lists a delegated row on its own", () => {
    const rows = sectionTree([{ agent: parent, device: null }, { agent: c1, device: null }], lookup, none);
    expect(rows.map((entry) => entry.agent.pane_id)).toEqual(["p"]);
  });

  it("draws an unfolded parent's descendants below it, deeper each level, in lineage order", () => {
    const open = new Map(all);
    open.set("p", { ...parent, lineage_collapsed: false });
    open.set("c1", { ...c1, lineage_collapsed: false });
    const rows = sectionTree([{ agent: open.get("p")!, device: null }], (_d, id) => open.get(id), none);
    expect(rows.map((entry) => [entry.agent.pane_id, entry.depth])).toEqual([
      ["p", 0],
      ["c1", 1],
      ["g", 2],
      ["c2", 1],
    ]);
  });

  it("counts every agent a heading speaks for, folded descendants included, each once", () => {
    const counts = new Map([["p", 3], ["c1", 1]]);
    const count = (_device: string | null, id: string) => counts.get(id) ?? 0;
    const other = row("o");
    const folded = sectionTree([{ agent: parent, device: null }, { agent: other, device: null }], lookup, count);
    expect(sectionCount(folded)).toBe(5);
    const open = new Map(all);
    open.set("p", { ...parent, lineage_collapsed: false });
    const unfolded = sectionTree([{ agent: open.get("p")!, device: null }, { agent: other, device: null }], (_d, id) => open.get(id), count);
    expect(sectionCount(unfolded)).toBe(5);
  });

  it("lists only the direct children still present for the popover", () => {
    const withGone = { ...parent, lineage_child_pane_ids: ["c1", "gone", "c2"] };
    expect(directChildren(withGone, (id) => all.get(id)).map((child) => child.pane_id)).toEqual(["c1", "c2"]);
  });
});

describe("the sidebar row's line (sidebar-readability D-4, B4, B6)", () => {
  it("draws a request and news from the start and never a quiet sentence, read or selected", () => {
    const asking = row("q", { demand: "approval", detail: "프로덕션 배포를 승인해 주세요", unread: false });
    expect(sidebarLine(asking)).toMatchObject({ text: "프로덕션 배포를 승인해 주세요", mode: "request" });
    expect(sidebarLine(row("n", { detail: "hide 앱을 다시 열었고 정상 실행 중", unread: true }))?.mode).toBe("news");
    expect(sidebarLine(row("s", { detail: "hide 앱을 다시 열었고 정상 실행 중", unread: false }))).toBeNull();
    expect(sidebarLine(row("e", { detail: "   " }))).toBeNull();
  });
});

describe("the Projects lineage fold (sidebar-readability D-6, B12)", () => {
  const walk = (rows: [AgentRow, number][]) => unfoldedRows(rows.map(([agent, depth]) => ({ agent, depth }))).map((drawn) => drawn.agent.pane_id);

  it("leaves out every descendant of a folded parent, the default, and draws the next root", () => {
    expect(walk([[row("a"), 0], [row("b"), 1], [row("c"), 2], [row("d"), 0]])).toEqual(["a", "d"]);
    expect(walk([[row("a", { lineage_collapsed: true }), 0], [row("b"), 1], [row("d"), 0]])).toEqual(["a", "d"]);
  });

  it("opens one level at a time: an unfolded parent shows its children and a folded child keeps its own", () => {
    const open = { lineage_collapsed: false };
    expect(walk([[row("a", open), 0], [row("b"), 1], [row("c"), 2], [row("e", open), 1], [row("f"), 2], [row("d"), 0]])).toEqual(["a", "b", "e", "f", "d"]);
  });
});

describe("the line's words (status-marks D-04, B7, B10, B13, B16)", () => {
  it("leads a stop to read and a kept line with how the turn ended, and leaves a block's cause alone", async () => {
    const { t } = await createInterfaceI18n("ko");
    const left = row("s", { activity: "stopped", mark: "unread", status_code: "unfinished", group: "done", unread: true, emphasized: true, detail: "테스트 3개 남음" });
    expect(sidebarLine(left)).toMatchObject({ mode: "news" });
    expect(lineText(t, left, sidebarLine(left)!)).toBe("일 남음 · 테스트 3개 남음");
    const kept = row("k", { activity: "stopped", mark: "quiet", status_code: "unfinished", group: "seen" });
    kept.state = { ...kept.state, line: { text: "테스트 3개 남음", mode: "kept", tone: { kind: "subtle", read: false } } };
    expect(lineText(t, kept, sidebarLine(kept)!)).toBe("일 남음 · 테스트 3개 남음");
    const finished = row("f", { activity: "stopped", mark: "done", status_code: "done", group: "done", unread: true, detail: "PR #12 올림" });
    expect(lineText(t, finished, sidebarLine(finished)!)).toBe("끝남 · PR #12 올림");
    const blocked = row("b", { demand: "error", activity: "stopped", mark: "turn", status_code: "error", group: "needs_you", emphasized: true, detail: "디스크 여유가 없음" });
    expect(lineText(t, blocked, sidebarLine(blocked)!)).toBe("막힘 · 디스크 여유가 없음");
  });

  it("leads a waiting row's sentence, or with none the cause Hide proved, with 대기 in every list (D-18, B10, B29)", async () => {
    const { t } = await createInterfaceI18n("ko");
    const said = row("w", { activity: "stopped", status_code: "waiting", wait: "background", detail: "빌드 끝나기를 기다림" });
    expect(lineText(t, said, sidebarLine(said)!)).toBe("대기 · 빌드 끝나기를 기다림");
    expect(treeLine(said)).toMatchObject({ mode: "waiting" });
    // A working row says its sentence alone, and a tree row draws none of it.
    const working = row("x", { detail: "파서 고치는 중", unread: true });
    expect(lineText(t, working, sidebarLine(working)!)).toBe("파서 고치는 중");
    expect(treeLine(working)).toBeNull();
    const waiting = (wait: AgentRow["wait"], pull_request?: number) => {
      const agent = row("w", { activity: "stopped", status_code: "waiting", wait, descendant_counts: { error: 0, approval: 0, question: 0, working: 2, done: 0 } });
      agent.state = { ...agent.state, line: { text: "", mode: "waiting", tone: { kind: "subtle", read: false }, pull_request } };
      return lineText(t, agent, sidebarLine(agent)!);
    };
    expect(waiting("checks", 12)).toBe("대기 · CI · PR #12");
    expect(waiting("children")).toBe("대기 · 자식 2명 작업 중");
    expect(waiting("background")).toBe("대기 · 백그라운드 작업");
    expect(waiting("reply")).toBe("대기 · 답장");
  });

  it("ends a finished turn's line with the tasks it left, kept apart from the sentence a narrow row cuts (D-19, B27)", async () => {
    const { t } = await createInterfaceI18n("ko");
    const left = row("d", { activity: "stopped", mark: "done", status_code: "done", group: "done", unread: true, detail: "배포 시작함", background_left: 2 });
    expect(lineParts(t, left, sidebarLine(left)!)).toEqual({ lead: "끝남", text: "배포 시작함", tail: "백그라운드 2" });
    expect(lineText(t, left, sidebarLine(left)!)).toBe("끝남 · 배포 시작함 · 백그라운드 2");
  });

  it("draws a sleeping row with its moon and no line, and a waking or failed one with its words, none dimmed (D-21, B31)", async () => {
    const { t } = await createInterfaceI18n("ko");
    const sleeping = row("z", { activity: "stopped", mark: "asleep", status_code: "sleeping", detail: "파서 고치는 중", unread: true });
    expect(sidebarLine(sleeping)).toBeNull();
    expect(rowDimmed(sleeping)).toBe(false);
    for (const [status_code, words] of [["waking", "깨우는 중"], ["sleep_failed", "재움 · 다시 열지 못함"]] as const) {
      const agent = row("z", { activity: "stopped", mark: "asleep", status_code });
      expect(lineText(t, agent, sidebarLine(agent)!)).toBe(words);
      expect(treeLine(agent)).not.toBeNull();
      expect(rowDimmed(agent)).toBe(false);
    }
    // The core names the unknown row; a resting one is not dimmed (sidebar-quiet-mark-indent D-04).
    expect(rowDimmed(row("u", { status_code: "unknown", mark: "unknown" }))).toBe(true);
    expect(rowDimmed(row("q", { activity: "stopped", status_code: "idle", mark: "quiet" }))).toBe(false);
    // The operator's turn outranks the moon (D-10): its ask draws as on any other row.
    const asked = row("y", { activity: "stopped", mark: "turn", status_code: "sleeping", demand: "question", detail: "어느 쪽으로 나눌까요?" });
    expect(sidebarLine(asked)).toMatchObject({ text: "어느 쪽으로 나눌까요?", mode: "request" });
  });

  it("names the pull request whose checks ended, in warning only when they failed", async () => {
    const { t } = await createInterfaceI18n("ko");
    const ended = (mode: "checks_passed" | "checks_failed") => {
      const agent = row("c", { activity: "stopped", mark: "unread", status_code: "stopped", group: "done", unread: true });
      agent.state = { ...agent.state, line: { text: "", mode, tone: { kind: mode === "checks_failed" ? "warning" : "news", read: false }, pull_request: 12 } };
      const line = sidebarLine(agent)!;
      return [lineText(t, agent, line), lineTone(line, agent)];
    };
    expect(ended("checks_passed")).toEqual(["CI 통과 · PR #12", "text-foreground"]);
    expect(ended("checks_failed")).toEqual(["CI 실패 · PR #12", "text-warning"]);
  });

  it("says in words that the work a stopped row waited for is gone, only on the sidebar (B16)", async () => {
    const { t } = await createInterfaceI18n("ko");
    const gone = row("g", { activity: "stopped", mark: "unread", status_code: "stopped", detail: "CI 끝나기를 기다림" });
    gone.state = { ...gone.state, line: { text: "", mode: "vanished", tone: { kind: "subtle", read: false } } };
    expect(lineText(t, gone, sidebarLine(gone)!)).toBe("멈춤 · 기다리던 작업이 사라짐");
  });
});
