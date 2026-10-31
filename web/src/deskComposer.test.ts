import { describe, expect, it } from "vitest";
import { chipDelete, chipStep, mentionAt, mentionTree, outgoing, pickMention, type MentionSession } from "./deskComposer";

const session = { kind: "session" as const, id: "w1:p2", name: "record-now", mark: "going" as const, agent_kind: "codex" };
const project = { kind: "project" as const, id: "/work/herdr-ide", name: "herdr-ide" };
const SESSION = "@session(w1:p2 record-now)";
const PROJECT = "@project(/work/herdr-ide herdr-ide)";

const row = (id: string, name: string, parent: string | null = null, mark: MentionSession["mark"] = "going"): MentionSession => ({
  id, name, agent_kind: "claude", mark, detail: null, parent, project: "herdr-ide", branch: "main", last_prompt_at_unix_ms: null,
});

describe("Desk's @ menu (B18)", () => {
  it("opens on an @ that starts a word and reads what follows it up to the caret", () => {
    expect(mentionAt("ask @rec", 8)).toEqual({ start: 4, query: "rec" });
    expect(mentionAt("@", 1)).toEqual({ start: 0, query: "" });
    expect(mentionAt("mail me@host", 12)).toBeNull();
    expect(mentionAt("ask @rec now", 12)).toBeNull();
    expect(mentionAt(`${SESSION}`, SESSION.length)).toBeNull();
  });
});

describe("the @ menu's session tree (D-84, B48)", () => {
  const sessions = [
    row("lead", "m21 holder-life"),
    row("worker", "pane 보관", "lead"),
    row("review", "리뷰", "lead", "quiet"),
    row("asker", "프로젝트 패널 인터뷰"),
    row("asking-child", "질문하는 자식", "asker", "turn"),
    row("deep", "손자 리뷰", "worker"),
    row("docs", "문서 리뷰"),
  ];
  const section = { sessions, projects: [{ id: "/work/herdr-ide", name: "herdr-ide", sessions: [] }, { id: "/work/creator", name: "creator", sessions: [] }] };
  const ids = (rows: { session: MentionSession }[]) => rows.map((one) => one.session.id);

  it("shows parents only, each with its child count, and opens a parent whose child asks", () => {
    const tree = mentionTree(section, "", {});
    expect(tree.rows.map((one) => [one.session.id, one.depth, one.children, one.open])).toEqual([
      ["lead", 0, 2, false],
      ["asker", 0, 1, true],
      ["asking-child", 1, 0, false],
      ["docs", 0, 0, false],
    ]);
    expect(tree.found).toEqual([]);
    expect(tree.projects).toHaveLength(2);
  });

  it("opens and closes a parent as the operator folds it, down to its own children", () => {
    expect(ids(mentionTree(section, "", { lead: true, asker: false }).rows)).toEqual(["lead", "worker", "review", "asker", "docs"]);
    expect(ids(mentionTree(section, "", { lead: true, worker: true }).rows)).toEqual(["lead", "worker", "deep", "review", "asker", "asking-child", "docs"]);
  });

  it("puts the parents a search finds first, then the children it finds with their parent's name", () => {
    const tree = mentionTree(section, "리뷰", {});
    expect(ids(tree.rows)).toEqual(["docs"]);
    expect(tree.found.map((one) => [one.session.id, one.crumb])).toEqual([["review", "m21 holder-life"], ["deep", "pane 보관"]]);
    expect(tree.projects).toEqual([]);
  });

  it("shows only the children group when no parent holds the words, and leaves out a child an opened parent shows", () => {
    expect(ids(mentionTree(section, "보관", {}).rows)).toEqual([]);
    expect(ids(mentionTree(section, "보관", {}).found)).toEqual(["worker"]);
    const opened = mentionTree(section, "m21", { lead: true });
    expect(ids(opened.rows)).toEqual(["lead", "worker", "review"]);
    expect(mentionTree(section, "HOLDER", {}).rows).toHaveLength(1);
  });
});

describe("chips in the box (D-85, B49)", () => {
  it("writes a picked mention as its token with one space after it and keeps it once", () => {
    const first = pickMention({ text: "ask @rec please", mentions: [] }, { start: 4, query: "rec" }, session);
    expect(first).toEqual({ draft: { text: `ask ${SESSION} please`, mentions: [session] }, caret: 4 + SESSION.length + 1 });
    expect(pickMention({ text: "@rec", mentions: [] }, { start: 0, query: "rec" }, session)).toEqual({ draft: { text: `${SESSION} `, mentions: [session] }, caret: SESSION.length + 1 });
    expect(pickMention(first.draft, { start: 0, query: "" }, session).draft.mentions).toEqual([session]);
  });

  it("takes a whole chip out with one ⌫ after it or ⌦ before it, and nothing beside plain words", () => {
    const draft = { text: `ask ${SESSION} now`, mentions: [session] };
    const end = 4 + SESSION.length;
    expect(chipDelete(draft, end, false)).toEqual({ draft: { text: "ask  now", mentions: [session] }, caret: 4 });
    expect(chipDelete(draft, 4, true)).toEqual({ draft: { text: "ask  now", mentions: [session] }, caret: 4 });
    expect(chipDelete(draft, end + 1, false)).toBeNull();
    expect(chipDelete(draft, 4, false)).toBeNull();
  });

  it("steps over a chip whole with ← after it and → before it", () => {
    const draft = { text: `ask ${SESSION} now`, mentions: [session] };
    expect(chipStep(draft, 4 + SESSION.length, false)).toBe(4);
    expect(chipStep(draft, 4, true)).toBe(4 + SESSION.length);
    expect(chipStep(draft, 2, true)).toBeNull();
  });

  it("does not take words the operator typed that only look like a token for a chip", () => {
    expect(chipDelete({ text: "@session(w1:p9 gone) x", mentions: [session] }, 20, false)).toBeNull();
  });
});

describe("what a Desk send carries (B49)", () => {
  it("sends the words with each chip as its token, and only the mentions still there, in their order", () => {
    expect(outgoing({ text: ` ${PROJECT} 에 ${SESSION} 붙여 ${SESSION} `, mentions: [session, project, { kind: "session", id: "w1:p9", name: "gone" }] })).toEqual({
      text: `${PROJECT} 에 ${SESSION} 붙여 ${SESSION}`,
      mentions: [project, session],
    });
  });
});
