// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import { clientI18n } from "../i18n/translator";
import type { Mission, MissionAttention, MissionCriterion, MissionNode, SnapshotRest } from "../snapshot";
import { useShellStore } from "../store";
import { AttentionChoices } from "./mission-attention";

// with-hide/hide pull request 2227: beside a raise's choices stands why its pull request
// is where it is, in the core's own words for the PRs list, and the choices
// are the core's list for the raise, drawn and never decided here.

afterEach(() => {
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

const line = (key: string, kind: MissionCriterion["kind"], state: MissionCriterion["state"]) => ({ key, kind, state, checkers: [{ worker: { agent: "claude", model: null, effort: null, effort_auto: false }, state, agent_id: "r" }] }) as unknown as MissionCriterion;

async function draw({ present, choices, criteria = [], reason = "worker_ended" }: { present: boolean; choices: MissionAttention["choices"]; criteria?: MissionCriterion[]; reason?: MissionAttention["reason"] }) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  // A pull request GitHub reads as green whose head conflicts with main: only the PRs list's reason says so.
  const rest = { navigator: { workspaces: [{ id: "w", pull_requests: [{ number: 7, title: "", url: "", badge: "open", review: null, is_draft: false, state: "mergeable" }], agent_scope: { prs: { groups: [], rows: [{ number: 7, group: "needs_you", fold: null, reason: { kind: "conflict", base: "main", since_unix_ms: 1 }, agents: [], agent_pane: null, checkout_id: null, checkout_ids: [] }] } } }] } } as unknown as SnapshotRest;
  useShellStore.setState({ rest, connection: "live" });
  const mission = { id: "m", workspace_id: "w", conductor: { present } } as unknown as Mission;
  const node = { key: "n", pr: 7, pr_state: "mergeable", external: null, criteria } as unknown as MissionNode;
  const attention: MissionAttention = { reason, pr: null, to: "operator", since_unix_ms: 5, on_row: false, item: true, choices };
  const sent: unknown[] = [];
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<AttentionChoices mission={mission} node={node} attention={attention} actions={createActions((event) => sent.push(event))} />));
  return { container, sent, unmount: () => act(async () => root.unmount()) };
}

it("says why the pull request stands where it does in the PRs list's words, and the failed line its CI does not", async () => {
  await clientI18n.changeLanguage("ko");
  const { container, unmount } = await draw({ present: true, choices: ["leave", "drop"], criteria: [line("ci", "ci", "fail"), line("review", "review", "fail")] });
  try {
    const pr = container.querySelector<HTMLElement>("[data-attention-pr]")!;
    expect(pr.dataset.attentionPrReason).toBe("conflict");
    expect(pr.textContent).toContain("충돌");
    expect(pr.textContent).not.toContain("머지 가능");
    // The CI line is the pull request's to say; the review is said by its kind alone.
    expect(container.querySelector("[data-attention-failed]")?.textContent).toBe("리뷰 실패");
    expect([...container.querySelectorAll<HTMLButtonElement>("[data-attention-choice]")].map((button) => button.textContent)).toEqual(["지휘 에이전트에게 넘기기", "이 단계 빼기"]);
  } finally {
    await unmount();
    await clientI18n.changeLanguage("en");
  }
});

// A line the plan wrote is a sentence that would push the choices off the row (operator report on M41): it is counted, and its sentence is the hover.
it("counts the failed lines the plan wrote and keeps their sentences for the hover", async () => {
  await clientI18n.changeLanguage("ko");
  const own = (key: string, text: string) => ({ ...line(key, "own", "fail"), text }) as MissionCriterion;
  const { container, unmount } = await draw({ present: true, choices: ["leave", "drop"], criteria: [own("c1", "업그레이드한 앱에서 기존 에이전트가 이어진다"), own("c2", "세 OS 표가 지원표와 맞는다"), line("review", "review", "fail")] });
  try {
    const failed = container.querySelector<HTMLElement>("[data-attention-failed]")!;
    expect(failed.textContent).toBe("리뷰 실패, 완료 기준 2개 실패");
    expect(failed.title.split("\n")).toEqual(["업그레이드한 앱에서 기존 에이전트가 이어진다 실패", "세 OS 표가 지원표와 맞는다 실패", "리뷰 실패"]);
  } finally {
    await unmount();
    await clientI18n.changeLanguage("en");
  }
});

it("draws the core's choices in its order, and leave as leave as is with no Conductor", async () => {
  const { container, sent, unmount } = await draw({ present: false, choices: ["leave", "attach"] });
  try {
    expect([...container.querySelectorAll<HTMLButtonElement>("[data-attention-choice]")].map((button) => [button.dataset.attentionChoice, button.textContent])).toEqual([["leave", "Leave as is"], ["attach", "Attach to node"]]);
    await act(async () => container.querySelector<HTMLButtonElement>('[data-attention-choice="leave"]')!.click());
    expect(sent).toEqual([{ schema_version: 2, kind: "mission_attention", payload: { mission: "m", node: "n", action: "leave", since_unix_ms: 5 } }]);
  } finally {
    await unmount();
  }
});

// A refused close leads with 완료로 닫기, which ends the mission with its empty lines passed, so it goes on a second press as 이 단계 빼기 does; the core offers no drop for it.
it("sends 완료로 닫기 only on its second press, and a press away puts it back", async () => {
  await clientI18n.changeLanguage("ko");
  const { container, sent, unmount } = await draw({ present: true, choices: ["done", "leave"], reason: "close_refused" });
  try {
    const choices = () => [...container.querySelectorAll<HTMLButtonElement>("[data-attention-choice]")].map((button) => [button.dataset.attentionChoice, button.textContent]);
    expect(choices()).toEqual([["done", "완료로 닫기"], ["leave", "지휘 에이전트에게 넘기기"]]);
    const done = () => container.querySelector<HTMLButtonElement>('[data-attention-choice="done"]')!;
    await act(async () => done().click());
    expect(sent).toEqual([]);
    expect(choices()[0]).toEqual(["done", "닫기 확인"]);
    await act(async () => { done().focus(); done().blur(); });
    expect(choices()[0]).toEqual(["done", "완료로 닫기"]);
    await act(async () => done().click());
    await act(async () => done().click());
    expect(sent).toEqual([{ schema_version: 2, kind: "mission_attention", payload: { mission: "m", node: "n", action: "done", since_unix_ms: 5 } }]);
  } finally {
    await unmount();
    await clientI18n.changeLanguage("en");
  }
});

it("does not ask twice for 끝으로 on a reason that is not a refused close", async () => {
  const { container, sent, unmount } = await draw({ present: true, choices: ["done"], reason: "merged_unproven" });
  try {
    await act(async () => container.querySelector<HTMLButtonElement>('[data-attention-choice="done"]')!.click());
    expect(sent).toHaveLength(1);
  } finally {
    await unmount();
  }
});
