import { describe, expect, it } from "vitest";
import { settleReply, type ReplyField } from "./agentReply";

// Issue 2513: a reply field keeps its words until the core says the pane took
// them, and reads only the answer to its own request.
describe("settleReply", () => {
  const out: ReplyField = { text: "네, 진행해 주세요", sent: "req-1", refused: null };

  it("waits while the answer is another request's or still sending", () => {
    expect(settleReply(out, null)).toBeNull();
    expect(settleReply(out, { request_id: "req-0", state: "refused", reason: "asleep" })).toBeNull();
    expect(settleReply(out, { request_id: "req-1", state: "sending" })).toBeNull();
    expect(settleReply({ ...out, sent: null }, { request_id: "req-1", state: "sent" })).toBeNull();
  });

  it("empties the field once its words were sent", () => {
    expect(settleReply(out, { request_id: "req-1", state: "sent" })).toEqual({ text: "", sent: null, refused: null });
  });

  it("keeps the words with the reason when they were refused", () => {
    expect(settleReply(out, { request_id: "req-1", state: "refused", reason: "pane_gone" })).toEqual({ text: "네, 진행해 주세요", sent: null, refused: "pane_gone" });
    expect(settleReply(out, { request_id: "req-1", state: "refused" })).toEqual({ text: "네, 진행해 주세요", sent: null, refused: "failed" });
  });
});
