import { describe, expect, it } from "vitest";
import { jobWorking, type DeskJob } from "../../desk";
import { jobLine, jobStateWords } from "./desk-job-card";

// Mission-115 (activity): a job's place for Desk and what its agent does are two facts, and its card says both.
// The words are the message keys, so the expected answer is which words the card picks, not their translation.
const t = ((key: string) => key) as unknown as Parameters<typeof jobLine>[0];

const job = (extra: Partial<DeskJob>): DeskJob => ({
  id: "intent-1", agent_id: "agent-1", pane_id: "w1:p2", title: "파리크라상 교육 자료 구성안", agent_kind: "claude", project: { workspace_id: "w", label: "creator" }, location: { kind: "worktree", label: "desk-parikrasang-plan" },
  state: "open", activity: "working", queue_position: null, now: null, plan: null, question: null, summary: null, pull_requests: [], outputs: [], joined: false,
  reason: null, failure: null, ask: null, mark: null, mark_read: false, machine: null, disconnected: false, changed_at_unix_ms: 0, read: false, mission: null, workers: [],
  ...extra,
});

describe("a job card's words", () => {
  it("says what a session moved out of Desk's group does after that it moved", () => {
    expect(jobLine(t, job({ state: "moved_out", now: "Drive에서 파일 찾는 중" }))).toBe("desk.job.moved_out · Drive에서 파일 찾는 중");
    expect(jobLine(t, job({ state: "moved_out", activity: "asking", ask: "question", question: "확정할까요?" }))).toBe("desk.job.moved_out · desk.ask.question · 확정할까요?");
    expect(jobLine(t, job({ state: "moved_out", activity: "done", summary: "구성안을 썼어요" }))).toBe("desk.job.moved_out · desk.job.done · 구성안을 썼어요");
    expect(jobStateWords(t, job({ state: "moved_out", activity: "working" }))).toBe("desk.job.moved_out · desk.job.working");
    // With no row left (its tab closed after it moved), the place is all there is to say.
    expect(jobLine(t, job({ state: "moved_out", activity: null }))).toBe("desk.job.moved_out");
  });

  it("says what an open session's agent does, from its row's activity", () => {
    expect(jobLine(t, job({ now: "캡처 중" }))).toBe("캡처 중");
    expect(jobLine(t, job({ activity: "asking", ask: "approval", question: "rm -rf dist" }))).toBe("desk.ask.approval · rm -rf dist");
    expect(jobLine(t, job({ activity: "sleeping" }))).toBe("desk.job.sleeping");
    // With no row to say it (its device not connected), an open job says nothing of its agent and is not counted at work.
    expect(jobLine(t, job({ activity: null }))).toBe("");
    expect(jobWorking(job({ activity: null }))).toBe(false);
    expect(jobLine(t, job({ activity: null, disconnected: true, now: "12분 전까지의 기록" }))).toBe("desk.job.disconnected · 12분 전까지의 기록");
  });
});
