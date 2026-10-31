import { describe, expect, it } from "vitest";
import { mentionAt, mentionRows, outgoing, pickMention } from "./deskComposer";

const session = { kind: "session" as const, id: "w1:p2", name: "record-now", mark: "going" as const, agent_kind: "codex" };
const project = { kind: "project" as const, id: "/work/herdr-ide", name: "herdr-ide" };

describe("Desk's @ menu (B18)", () => {
  it("opens on an @ that starts a word and reads what follows it up to the caret", () => {
    expect(mentionAt("ask @rec", 8)).toEqual({ start: 4, query: "rec" });
    expect(mentionAt("@", 1)).toEqual({ start: 0, query: "" });
    expect(mentionAt("mail me@host", 12)).toBeNull();
    expect(mentionAt("ask @rec now", 12)).toBeNull();
  });

  it("filters sessions and projects alike by name, case aside", () => {
    const rows = mentionRows({ sessions: [{ id: "a", name: "Record-now", agent_kind: "codex", mark: "going", detail: null }, { id: "b", name: "gallery", agent_kind: "claude", mark: "turn", detail: null }], projects: [{ id: "/p", name: "recorder", sessions: [] }] }, "rec");
    expect(rows.sessions.map((row) => row.id)).toEqual(["a"]);
    expect(rows.projects.map((row) => row.id)).toEqual(["/p"]);
  });

  it("writes a picked session as @name with one space after it and keeps it once", () => {
    const first = pickMention({ text: "ask @rec please", mentions: [] }, { start: 4, query: "rec" }, session);
    expect(first).toEqual({ draft: { text: "ask @record-now please", mentions: [session] }, caret: 16 });
    expect(pickMention({ text: "@rec", mentions: [] }, { start: 0, query: "rec" }, session)).toEqual({ draft: { text: "@record-now ", mentions: [session] }, caret: 12 });
    const again = pickMention(first.draft, { start: 0, query: "" }, session);
    expect(again.draft.mentions).toEqual([session]);
  });
});

describe("what a Desk send carries (B18)", () => {
  it("turns each picked mention still written into its token, and drops one the operator deleted", () => {
    expect(outgoing({ text: " @record-now, look at @herdr-ide ", mentions: [session, project, { kind: "session", id: "w1:p9", name: "gone" }] })).toEqual({
      text: "@session(w1:p2 record-now), look at @project(/work/herdr-ide herdr-ide)",
      mentions: [session, project],
    });
  });

  it("does not take a name inside a longer word for the mention", () => {
    const short = { kind: "session" as const, id: "w1:p3", name: "record" };
    expect(outgoing({ text: "@record-now then @record", mentions: [short] })).toEqual({ text: "@record-now then @session(w1:p3 record)", mentions: [short] });
  });
});
