import { describe, expect, it } from "vitest";
import { keepSessionDraft, NO_SESSION_DRAFTS, SESSION_DRAFTS_KEPT, sessionDraft } from "./sessionDrafts";

// Operator request 2026-11-27 (mission-109): a session's input keeps its words per pane, capped, the pane written in
// longest ago let go first, and an emptied box (a send that landed) forgets its pane.
describe("the words kept per session's input", () => {
  it("gives each pane its own words and forgets a pane whose box is emptied", () => {
    const kept = keepSessionDraft(keepSessionDraft(NO_SESSION_DRAFTS, "w1:p2", "다크도"), "w1:p3", "라이트");
    expect([sessionDraft(kept, "w1:p2"), sessionDraft(kept, "w1:p3"), sessionDraft(kept, "w1:p9")]).toEqual(["다크도", "라이트", ""]);
    const sent = keepSessionDraft(kept, "w1:p2", "");
    expect(sessionDraft(sent, "w1:p2")).toBe("");
    expect(sessionDraft(sent, "w1:p3")).toBe("라이트");
    // Emptying a box that kept nothing leaves the very same list.
    expect(keepSessionDraft(sent, "w1:p2", "")).toBe(sent);
  });

  it("keeps the panes written in most recently up to the cap, letting the oldest go first", () => {
    let kept = NO_SESSION_DRAFTS;
    for (let index = 0; index < SESSION_DRAFTS_KEPT; index += 1) kept = keepSessionDraft(kept, `p${index}`, `words ${index}`);
    // Writing in the first pane again makes it the newest, so the second is the oldest when one more comes.
    kept = keepSessionDraft(kept, "p0", "words 0 again");
    kept = keepSessionDraft(kept, "new", "new words");
    expect(kept).toHaveLength(SESSION_DRAFTS_KEPT);
    expect([sessionDraft(kept, "p0"), sessionDraft(kept, "p1"), sessionDraft(kept, "new")]).toEqual(["words 0 again", "", "new words"]);
  });
});
