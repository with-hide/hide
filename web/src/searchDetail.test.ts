import { describe, expect, it } from "vitest";
import { PR_260, PR_275, READ_AT, RICH, requestWith } from "./gallery/cmdkSceneData";
import { initializeInterfaceI18n } from "./i18n/instance";
import { searchEntries } from "./search";
import { detailOf } from "./searchDetail";

const now = READ_AT + 5 * 60_000;

describe("the palette's detail pane in the operator's language", () => {
  it("words a checkout's facts and action", () => {
    const { t } = initializeInterfaceI18n("ko");
    const entry = searchEntries(RICH, t).find((row) => row.id === "checkout:c-sand")!;
    const detail = detailOf(RICH, entry, now, t, "ko");
    expect(detail.kind).toBe("체크아웃");
    expect(detail.facts).toEqual([
      ["변경", "기준 이후 커밋 1개 · 변경 파일 14개"],
      ["PR", "#275"],
      ["이슈", "#273"],
    ]);
    expect(detail.action).toBe("체크아웃 열기");
  });

  it("words a pull request's state, facts and action", () => {
    const { t } = initializeInterfaceI18n("ja");
    const entry = searchEntries(RICH, t).find((row) => row.id === "pr:w1:275")!;
    const detail = detailOf(RICH, entry, now, t, "ja");
    expect(detail.kind).toBe("PR");
    expect(detail.facts.map(([label]) => label)).toEqual(["ブランチ", "閉じる Issue", "読み込み"]);
    expect(detail.facts.at(-1)?.[1]).toBe("5 分前に読み込み");
    expect(detail.action).toBe("PR ビューで開く");
  });

  it("names the pull requests the core links to an agent, and has no line for an agent with none", () => {
    const { t } = initializeInterfaceI18n("ko");
    const tied = structuredClone(RICH) as typeof RICH;
    tied.navigator!.agents = tied.navigator!.agents!.map((row) => (row.pane_id === "p-dag" ? { ...row, request: requestWith("review", [PR_275, PR_260]) } : row));
    const agentFacts = (pane: string) => detailOf(tied, searchEntries(tied, t).find((row) => row.id === `agent:${pane}`)!, now, t, "ko").facts;
    expect(agentFacts("p-child")).toContainEqual(["PR", "#275"]);
    expect(agentFacts("p-dag")).toContainEqual(["PR", "#275, #260"]);
    expect(agentFacts("p-parent").map(([label]) => label)).not.toContain("PR");
  });

  it("counts a pull request's recorded sessions and leaves the line out at none", () => {
    const { t } = initializeInterfaceI18n("ko");
    const entry = searchEntries(RICH, t).find((row) => row.id === "pr:w1:275")!;
    const links = { projects: { w1: { prs: { "275": 3 } } }, filling: false };
    expect(detailOf(RICH, entry, now, t, "ko", links).facts).toContainEqual(["세션", "3"]);
    expect(detailOf(RICH, entry, now, t, "ko", { projects: {}, filling: false }).facts.map(([label]) => label)).not.toContain("세션");
  });
});
