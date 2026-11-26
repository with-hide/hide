import { describe, expect, it } from "vitest";
import { chipDelete, chipStep, commandAt, commandRows, keepSent, mentionAt, mentionTree, outgoing, pickCommand, pickMention, sentAttachment, sentWords, traySendable, trayEntries, type MentionSession, type TrayFile } from "./deskComposer";

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

describe("Desk's / menu", () => {
  const commands = [
    { name: "/review", description: "review the diff" },
    { name: "/compact", description: "compact the context", hint: "what to keep" },
    { name: "$codex-review", description: "a skill" },
    { name: "/context", description: "show the context" },
  ];
  const names = (rows: { name: string }[]) => rows.map((one) => one.name);

  it("opens only on a / that starts the box, up to the caret, before any space", () => {
    expect(commandAt("/co", 3)).toEqual({ query: "co" });
    expect(commandAt("/", 1)).toEqual({ query: "" });
    expect(commandAt("/compact now", 12)).toBeNull();
    expect(commandAt("/compact now", 3)).toEqual({ query: "co" });
    expect(commandAt("say /co", 7)).toBeNull();
    expect(commandAt(" /co", 4)).toBeNull();
  });

  it("filters by the name without its /, case aside, names that start with the words first", () => {
    expect(names(commandRows(commands, ""))).toEqual(["/review", "/compact", "$codex-review", "/context"]);
    expect(names(commandRows(commands, "CO"))).toEqual(["/compact", "/context", "$codex-review"]);
    expect(names(commandRows(commands, "review"))).toEqual(["/review", "$codex-review"]);
    expect(commandRows(commands, "nothing")).toEqual([]);
  });

  it("writes the picked command and one space over the / word and keeps the words after it", () => {
    const draft = (text: string) => ({ text, mentions: [] });
    expect(pickCommand(draft("/co"), { query: "co" }, commands[1]!)).toEqual({ draft: draft("/compact "), caret: 9 });
    expect(pickCommand(draft("/co keep the plan"), { query: "co" }, commands[1]!)).toEqual({ draft: draft("/compact keep the plan"), caret: 9 });
    expect(pickCommand(draft("/"), { query: "" }, commands[2]!)).toEqual({ draft: draft("$codex-review "), caret: 14 });
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
    expect(chipDelete(draft, end + 2, false)).toBeNull();
    expect(chipDelete(draft, 4, false)).toBeNull();
  });

  it("takes the space a pick put after the chip with it, so one ⌫ right after a pick leaves the words as they were", () => {
    const picked = pickMention({ text: "ask @rec", mentions: [] }, { start: 4, query: "rec" }, session);
    expect(chipDelete(picked.draft, picked.caret, false)).toEqual({ draft: { text: "ask ", mentions: [session] }, caret: 4 });
    const middle = { text: `ask ${SESSION} now`, mentions: [session] };
    expect(chipDelete(middle, 4 + SESSION.length + 1, false)).toEqual({ draft: { text: "ask now", mentions: [session] }, caret: 4 });
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

describe("a composer's tray (desk-2-ui B41, B42)", () => {
  const MIB20 = 20 * 1024 * 1024;

  it("takes a file of 20 MiB to read and holds one byte past it red and unread", () => {
    expect(trayEntries([], [{ id: "a", name: "edge.bin", size: MIB20 }, { id: "b", name: "big.png", size: MIB20 + 1 }]).map((one) => one.state)).toEqual(["reading", "too_large"]);
  });

  it("reads no file a message could not carry: past 8 files or 40 MiB with the ones it holds, the rest are held red and unread", () => {
    const held = (id: string, state: TrayFile["state"], size: number): TrayFile => ({ id, name: id, size, preview: null, state, read: 0 });
    const photos = Array.from({ length: 50 }, (_, index) => ({ id: `p${index}`, name: `p${index}.jpg`, size: 1024 }));
    const fifty = trayEntries([], photos).map((one) => one.state);
    expect(fifty.filter((state) => state === "reading")).toHaveLength(8);
    expect(fifty.slice(8).every((state) => state === "over_batch")).toBe(true);
    // Bytes count what the tray already holds; a file past 20 MiB or a red one does not.
    const holding = [held("a", "ready", MIB20), held("x", "too_large", MIB20 + 1), held("y", "over_batch", 1)];
    expect(trayEntries(holding, [{ id: "b", name: "b", size: MIB20 }, { id: "c", name: "c", size: 1 }]).map((one) => one.state)).toEqual(["reading", "over_batch"]);
  });

  it("sends every file once all are read, and holds the send while one reads, is past the cap or could not be read", () => {
    const file = (id: string, state: TrayFile["state"]): TrayFile => ({ id, name: id, size: 1, preview: null, state, read: 0 });
    expect(traySendable([])).toEqual([]);
    expect(traySendable([file("a", "ready"), file("b", "ready")])?.map((one) => one.id)).toEqual(["a", "b"]);
    for (const held of ["reading", "too_large", "over_batch", "unreadable"] as const) expect(traySendable([file("a", "ready"), file("b", held)])).toBeNull();
  });

  it("names a sent file by the name the operator's file had, whichever machine staged it", () => {
    const stage = "0b7a3c52-9a1e-4f7d-8c2b-1a2b3c4d5e6f";
    expect(sentAttachment(`/Users/me/.hide/state/attachments/${stage}-0-화면 1.png`, 10)).toEqual({ name: "화면 1.png", size: 10, preview: null });
    expect(sentAttachment(`C:\\Users\\me\\.hide\\state\\attachments\\${stage}-1-a-b.log`).name).toBe("a-b.log");
    // A device stages a file under a name Hide makes; this window knows the file it sent under that stage.
    expect(sentAttachment(`C:/Users/me/.hide/state/terminal-attachments/hide-${stage}-1.log`).name).toBe(`hide-${stage}-1.log`);
    keepSent(`${stage}-1`, "설치 로그.log", null);
    expect(sentAttachment(`C:/Users/me/.hide/state/terminal-attachments/hide-${stage}-1.log`).name).toBe("설치 로그.log");
    expect(sentAttachment(`/home/me/notes-${stage}.md`).name).toBe(`notes-${stage}.md`);
    expect(sentWords(`"/s/attachments/${stage}-0-x.png" 봐 줘`)).toEqual({ files: [{ name: "x.png", size: null, preview: null }], words: "봐 줘" });
    expect(sentWords("그냥 말")).toEqual({ files: [], words: "그냥 말" });
  });
});
