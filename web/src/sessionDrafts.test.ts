import { describe, expect, it } from "vitest";
import { keepSessionDraft, NO_SESSION_DRAFTS, sendSessionDraft, SESSION_DRAFT_BYTES, SESSION_DRAFTS_KEPT, sessionDraft, sessionDraftsLeft, sessionDraftTakes, settleSessionSend, uncertainSessionSends, visitSessionTerminal } from "./sessionDrafts";
import type { RemoteStatus } from "./snapshot";

// Operator request 2026-11-27 (mission-109): a session's input keeps its words per pane, capped, the pane written in
// longest ago let go first, and an emptied box (a send that landed) forgets its pane.
describe("the words kept per session's input", () => {
  it("gives each pane its own words and forgets a pane whose box is emptied", () => {
    const kept = keepSessionDraft(keepSessionDraft(NO_SESSION_DRAFTS, "w1:p2", null, "다크도"), "w1:p3", null, "라이트");
    expect([sessionDraft(kept, "w1:p2"), sessionDraft(kept, "w1:p3"), sessionDraft(kept, "w1:p9")]).toEqual(["다크도", "라이트", ""]);
    const sent = keepSessionDraft(kept, "w1:p2", null, "");
    expect(sessionDraft(sent, "w1:p2")).toBe("");
    expect(sessionDraft(sent, "w1:p3")).toBe("라이트");
    // Emptying a box that kept nothing leaves the very same list.
    expect(keepSessionDraft(sent, "w1:p2", null, "")).toBe(sent);
  });

  it("keeps the panes written in most recently up to the cap, letting the oldest go first", () => {
    let kept = NO_SESSION_DRAFTS;
    for (let index = 0; index < SESSION_DRAFTS_KEPT; index += 1) kept = keepSessionDraft(kept, `p${index}`, null, `words ${index}`);
    // Writing in the first pane again makes it the newest, so the second is the oldest when one more comes.
    kept = keepSessionDraft(kept, "p0", null, "words 0 again");
    kept = keepSessionDraft(kept, "new", null, "new words");
    expect(kept).toHaveLength(SESSION_DRAFTS_KEPT);
    expect([sessionDraft(kept, "p0"), sessionDraft(kept, "p1"), sessionDraft(kept, "new")]).toEqual(["words 0 again", "", "new words"]);
  });

  it("takes an edit within one send, or one that shortens a box already past it, and never one that lengthens past it", () => {
    const fits = "가".repeat(SESSION_DRAFT_BYTES / 3);
    expect(sessionDraftTakes(NO_SESSION_DRAFTS, "w1:p2", fits)).toBe(true);
    // One more three-byte character no longer fits.
    expect(sessionDraftTakes(NO_SESSION_DRAFTS, "w1:p2", `${fits}가`)).toBe(false);
    const over = keepSessionDraft(NO_SESSION_DRAFTS, "w1:p2", null, `${fits}가가`);
    expect(sessionDraftTakes(over, "w1:p2", `${fits}가`)).toBe(true);
    expect(sessionDraftTakes(over, "w1:p2", `${fits}가가가`)).toBe(false);
  });

  it("lets a pane's words go once its own machine no longer lists it, and keeps a device's while that device is not connected", () => {
    let kept = keepSessionDraft(NO_SESSION_DRAFTS, "w1:p2", null, "here");
    kept = keepSessionDraft(kept, "remote:mini:pane:w2:p7", "mini", "there");
    const device = (state: string, panes: string[]): RemoteStatus[] => [{ target_id: "mini", state, message: null, session: { agents: panes.map((pane_id) => ({ pane_id })) } } as unknown as RemoteStatus];
    // This machine still lists its pane and the device, cut off, says nothing of its own.
    expect(sessionDraftsLeft(kept, [{ pane_id: "w1:p2" }], device("not_connected", []))).toBe(kept);
    expect(sessionDraftsLeft(kept, [{ pane_id: "w1:p2" }], undefined)).toBe(kept);
    // Connected again without the pane, the device's draft goes; this machine's pane closed, its draft goes.
    expect(sessionDraftsLeft(kept, [{ pane_id: "w1:p2" }], device("connected", []))).toEqual([{ pane: "w1:p2", device: null, text: "here" }]);
    expect(sessionDraftsLeft(kept, [], device("connected", ["remote:mini:pane:w2:p7"]))).toEqual([{ pane: "remote:mini:pane:w2:p7", device: "mini", text: "there" }]);
  });

  it("keeps a send's words until the pane answers: taken they go, refused they come back before words written since", () => {
    const sent = sendSessionDraft(NO_SESSION_DRAFTS, "w1:p2", null, "r1", "보낸 말");
    expect(sessionDraft(sent, "w1:p2")).toBe("");
    // The pane took them: nothing is left for the box, and the pane is forgotten.
    expect(settleSessionSend(sent, "r1", true)).toEqual([]);
    // Refused after new words were written: both stay, the refused ones first.
    const written = keepSessionDraft(sent, "w1:p2", null, "새 말");
    expect(sessionDraft(settleSessionSend(written, "r1", false), "w1:p2")).toBe("보낸 말\n새 말");
    // An answer to another request changes nothing.
    expect(settleSessionSend(sent, "r9", false)).toBe(sent);
  });

  it("cuts nothing when refused words come back on top of words written since", () => {
    const full = "x".repeat(SESSION_DRAFT_BYTES);
    const written = keepSessionDraft(sendSessionDraft(NO_SESSION_DRAFTS, "w1:p2", null, "r1", full), "w1:p2", null, "new words");
    expect(sessionDraft(settleSessionSend(written, "r1", false), "w1:p2")).toBe(`${full}\nnew words`);
  });
});

it("preserves words and marks lost completions uncertain, including a model change with an existing draft", () => {
  const waiting = sendSessionDraft(NO_SESSION_DRAFTS, "p", null, "r", "reply");
  const uncertain = uncertainSessionSends(keepSessionDraft(waiting, "p", null, "next"));
  expect(uncertain).toEqual([{ pane: "p", device: null, text: "reply\nnext", uncertain: true }]);
  expect(visitSessionTerminal(uncertain, "p")).toEqual([{ pane: "p", device: null, text: "reply\nnext" }]);
  const model = sendSessionDraft(keepSessionDraft(NO_SESSION_DRAFTS, "p", null, "unsent"), "p", null, "model", "", true);
  expect(sessionDraft(settleSessionSend(model, "model", false), "p")).toBe("unsent");
  expect(uncertainSessionSends(model)).toEqual([{ pane: "p", device: null, text: "unsent", uncertain: true }]);
});
