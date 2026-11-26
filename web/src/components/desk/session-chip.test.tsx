// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { DeskJob, DeskMention, DeskSessionRef, SessionFacts } from "../../desk";
import type { SnapshotRest } from "../../snapshot";
import { useShellStore } from "../../store";
import { TooltipProvider } from "../ui/tooltip";
import { DeskReply } from "./desk-chat";
import { SessionCard, SessionChipProvider } from "./session-chip";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });

const card: DeskJob = {
  id: "pane:w1:p7", agent_id: null, pane_id: "w1:p7", title: "agent-journey", agent_kind: "codex", project: { workspace_id: "w", label: "herdr-ide" }, location: { kind: "worktree", label: "journey-agy" },
  state: "working", queue_position: null, now: "Linux e2e 결과 기다리는 중", plan: { done: 3, total: 5 }, question: null, summary: null, pull_requests: [{ number: 1921, state: "pending" }],
  outputs: [{ id: "o1", kind: "image", label: "a.png", thumb: null, seen: true, pr_state: null, device_id: null, checkout: null, path: null, url: null }, { id: "o2", kind: "doc", label: "b.md", thumb: null, seen: true, pr_state: null, device_id: null, checkout: null, path: null, url: null }],
  joined: false, reason: null, failure: null, ask: null, mark: "going", mark_read: false, machine: null, disconnected: false, changed_at_unix_ms: 0, read: false, mission: null, workers: [],
};

let mounted: ReturnType<typeof createRoot>[] = [];

beforeEach(() => vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true));
afterEach(async () => {
  for (const root of mounted) await act(async () => root.unmount());
  mounted = [];
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
});

async function render(node: React.ReactNode): Promise<HTMLElement> {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  mounted.push(root);
  await act(async () => root.render(<TooltipProvider>{node}</TooltipProvider>));
  // The reply's chips are drawn in roots of their own, mounted as its Markdown is laid out.
  await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
  return host;
}

it("draws every session a reply names as a chip, the seventeenth on too, and dims one no longer listed (B50)", async () => {
  const mentions: DeskMention[] = Array.from({ length: 20 }, (_, index) => ({ kind: "session", id: `w1:p${index + 1}`, name: `job-${index + 1}`, mark: "going", agent_kind: "claude" }));
  const text = mentions.map((mention) => `@session(${mention.id} ${mention.name})`).join(" ");
  const facts = (session: DeskSessionRef): SessionFacts | null =>
    session.pane_id === "w1:p20" ? { listed: false, pane_id: null, name: null, mark: null, agent_kind: null, card: null, since: null, parent: null, record: null } : { listed: true, pane_id: session.pane_id ?? null, name: null, mark: "going", agent_kind: "claude", card: null, since: null, parent: null, record: null };
  const host = await render(<SessionChipProvider source={{ facts, onOpen: () => undefined }}><DeskReply text={text} mentions={mentions} /></SessionChipProvider>);
  const chips = [...host.querySelectorAll("[data-desk-session-chip]")].map((chip) => chip.getAttribute("data-desk-session-chip"));
  expect(chips).toEqual(mentions.map((mention) => mention.name));
  expect(host.textContent).not.toContain("@session(");
  expect([...host.querySelectorAll('[data-desk-session-gone="true"]')].map((chip) => chip.textContent)).toEqual(["job-20"]);
});

it("names a session by its sidebar row while the sidebar lists it, whatever id the reply wrote, and by the reply's words once it is gone", async () => {
  const mentions: DeskMention[] = [
    { kind: "session", id: "w1:p1", name: "agent-47656", mark: null, agent_kind: null },
    { kind: "session", id: "w1:p2", name: "auto-ol4484-r10-o4", mark: null, agent_kind: null },
  ];
  const text = mentions.map((mention) => `@session(${mention.id} ${mention.name})`).join(" and ");
  const facts = (session: DeskSessionRef): SessionFacts | null =>
    session.pane_id === "w1:p1" ? { listed: true, pane_id: "w1:p1", name: "OneDrive 충돌 분석", mark: "turn", agent_kind: "codex", card: null, since: null, parent: null, record: null } : { listed: false, pane_id: null, name: null, mark: null, agent_kind: null, card: null, since: null, parent: null, record: null };
  const host = await render(<SessionChipProvider source={{ facts, onOpen: () => undefined }}><DeskReply text={text} mentions={mentions} /></SessionChipProvider>);
  expect([...host.querySelectorAll("[data-desk-session-chip]")].map((chip) => chip.textContent)).toEqual(["OneDrive 충돌 분석", "auto-ol4484-r10-o4"]);
  expect(host.textContent).not.toContain("agent-47656");
});

it("shows a session's card with its place, line, plan, pull request, outputs and parent, and opens its thread (D-86)", async () => {
  const opened: string[] = [];
  const parent: DeskSessionRef = { name: "agent-14586", mark: "turn", agent_kind: "claude", pane_id: "w1:p0" };
  const host = await render(<SessionCard job={card} since={null} parent={parent} onOpen={() => opened.push("thread")} onParent={(session) => opened.push(session.name)} />);
  const words = host.textContent ?? "";
  for (const fact of ["agent-journey", "herdr-ide · journey-agy", "Linux e2e 결과 기다리는 중", "3/5", "agent-14586"]) expect(words).toContain(fact);
  expect(host.querySelector("[data-desk-session-card-pr]")?.getAttribute("data-desk-session-card-pr")).toBe("1921");
  expect(host.querySelector("[data-desk-session-card-outputs]")?.getAttribute("data-desk-session-card-outputs")).toBe("2");
  await act(async () => host.querySelector<HTMLElement>("[data-desk-session-card-open]")!.click());
  await act(async () => host.querySelector<HTMLElement>('[data-desk-session-card-parent] [data-desk-session-chip]')!.click());
  expect(opened).toEqual(["thread", "agent-14586"]);
});

it("draws each mission the reply item carries in place of its token, a title with parentheses whole, and leaves the text of one it does not carry", async () => {
  const missions = [{ id: "mission-63", title: "결제 흐름 정리 (11-25)" }];
  const text = "@mission(mission-63 결제 흐름 정리 (11-25)) runs; @mission(mission-9 not read) waits.";
  const host = await render(<DeskReply text={text} missions={missions} />);
  const drawn = [...host.querySelectorAll("[data-desk-mission-mention]")];
  expect(drawn.map((one) => [one.getAttribute("data-desk-mission-mention"), one.textContent])).toEqual([["mission-63", "결제 흐름 정리 (11-25)"]]);
  expect(host.textContent).not.toContain("@mission(mission-63");
  expect(host.textContent).toContain("@mission(mission-9 not read)");
});

it("draws a session named inside a table cell, a list item and a quote as a chip, never as its token", async () => {
  const mentions: DeskMention[] = ["w1:p1", "w1:p2", "w1:p3"].map((id, index) => ({ kind: "session", id, name: `job-${index + 1}`, mark: "going", agent_kind: "claude" }));
  const [cell, item, quote] = mentions.map((mention) => `@session(${mention.id} ${mention.name})`);
  const text = `| 역할 | 세션 |\n| --- | --- |\n| 지휘자 | ${cell} |\n\n- 맡은 쪽: ${item}\n\n> 기다림: ${quote}`;
  const facts = (session: DeskSessionRef): SessionFacts => ({ listed: true, pane_id: session.pane_id ?? null, name: null, mark: "going", agent_kind: "claude", card: null, since: null, parent: null, record: null });
  const host = await render(<SessionChipProvider source={{ facts, onOpen: () => undefined }}><DeskReply text={text} mentions={mentions} /></SessionChipProvider>);
  // The table is drawn once the lazily loaded Markdown language parsed the reply.
  await act(() => vi.waitFor(() => expect(host.querySelector(".cm-md-table [data-desk-session-chip]")).not.toBeNull()));
  expect(host.querySelector(".cm-md-table [data-desk-session-chip]")?.textContent).toBe("job-1");
  expect([...host.querySelectorAll("[data-desk-session-chip]")].map((chip) => chip.textContent).sort()).toEqual(["job-1", "job-2", "job-3"]);
  expect(host.textContent).not.toContain("@session(");
});

it("puts the chips of the missions a paragraph names in one row right behind it, not in its words, and none for a mission the app does not hold", async () => {
  const saved = useShellStore.getState();
  const held = { id: "mission-63", title: "결제 흐름 정리", status: "running", nodes: [], conductor: { present: true }, decision: null, question: null, delay: null, closed_at_unix_ms: null };
  useShellStore.setState({ rest: { missions: { items: [held] } } as unknown as SnapshotRest });
  try {
    const missions = [{ id: "mission-63", title: "결제 흐름 정리" }, { id: "mission-9", title: "모르는 미션" }];
    const text = "@mission(mission-63 결제 흐름 정리) and @mission(mission-9 모르는 미션) run.\n\nSecond paragraph names no mission.";
    const host = await render(<DeskReply text={text} missions={missions} onMission={() => {}} />);
    const rows = [...host.querySelectorAll("[data-markdown-chip-row]")];
    expect(rows.map((row) => [row.getAttribute("data-markdown-chip-row"), row.querySelectorAll("[data-mission-chip]").length])).toEqual([["0", 1]]);
    // The paragraph's words keep both titles and hold no chip; the row follows its line and comes before the next paragraph.
    const line = host.querySelector(".cm-line")!;
    expect([line.textContent, line.querySelector("[data-mission-chip]")]).toEqual(["결제 흐름 정리 and 모르는 미션 run.", null]);
    expect(rows[0]!.compareDocumentPosition(host.querySelectorAll(".cm-line")[2]!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  } finally {
    useShellStore.setState(saved, true);
  }
});

it("redraws no answer when a mission changes unless the answer names that mission, so a selection in it stays", async () => {
  const saved = useShellStore.getState();
  const held = (title: string) => ({ id: "mission-63", title, status: "running", nodes: [], conductor: { present: true }, decision: null, question: null, delay: null, closed_at_unix_ms: null });
  useShellStore.setState({ rest: { missions: { items: [held("결제 흐름 정리")] } } as unknown as SnapshotRest });
  try {
    const host = await render(<><DeskReply text="No mission here." /><DeskReply text="@mission(mission-63 결제 흐름 정리) runs." missions={[{ id: "mission-63", title: "결제 흐름 정리" }]} onMission={() => {}} /></>);
    const editors = () => [...host.querySelectorAll(".cm-editor")];
    const before = editors();
    expect(before).toHaveLength(2);
    // A mission changes (its title, a node): both answers keep their editors, so a text selection in either is not lost.
    await act(async () => useShellStore.setState({ rest: { missions: { items: [held("결제 흐름 정리 (개정)")] } } as unknown as SnapshotRest }));
    expect(editors().every((editor, at) => editor === before[at])).toBe(true);
    // The chip itself follows the store, in its own root.
    expect(host.querySelector("[data-mission-chip]")?.textContent).toContain("개정");
  } finally {
    useShellStore.setState(saved, true);
  }
});

it("marks the chip of the session open beside Desk as the open one, and no other (frame J, 가-2 and 나-1)", async () => {
  const mentions: DeskMention[] = [
    { kind: "session", id: "w1:p1", name: "Windows 설치 폴더 OneDrive 문제", mark: "going", agent_kind: "codex" },
    { kind: "session", id: "w1:p2", name: "Mac mini 연결 검증", mark: "going", agent_kind: "claude" },
  ];
  const text = mentions.map((mention) => `@session(${mention.id} ${mention.name})`).join(" ");
  const facts = (session: DeskSessionRef): SessionFacts => ({ listed: true, pane_id: session.pane_id ?? null, name: null, mark: "going", agent_kind: "codex", card: null, since: null, parent: null, record: null });
  const open = (pane: string | null) => render(<SessionChipProvider source={{ facts, onOpen: () => undefined, open: pane }}><DeskReply text={text} mentions={mentions} onMention={() => undefined} /></SessionChipProvider>);
  const marked = (host: HTMLElement) => [...host.querySelectorAll('[data-desk-session-open="true"]')].map((chip) => chip.textContent);
  expect(marked(await open("w1:p1"))).toEqual(["Windows 설치 폴더 OneDrive 문제"]);
  document.body.innerHTML = "";
  expect(marked(await open("w1:p9"))).toEqual([]);
});
