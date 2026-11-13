// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { REPLY_FIELDS_MAX, REPLY_RECEIPT_WAIT_MS, replyReceipt, settleReply, useAgentReply, useReplyFields } from "./agentReply";
import { legacyAgentRow } from "../test/legacyAgentRow";
import { useShellStore } from "./store";

// Issue 2513: a reply field keeps its words until the core's receipt for its
// own request says the pane took them, never waits for ever on a receipt that
// will not come, and holds a bounded number of drafts of live panes.
const savedShell = useShellStore.getState();
beforeEach(() => useReplyFields.setState({ fields: {} }));
afterEach(() => {
  useShellStore.setState(savedShell, true);
  vi.useRealTimers();
  vi.unstubAllGlobals();
  document.body.replaceChildren();
});

describe("settleReply", () => {
  const out = { text: "네, 진행해 주세요", sent: "req-1" };

  it("waits while the receipt is another request's or still sending", () => {
    expect(settleReply(out, null)).toBeNull();
    expect(settleReply(out, { request_id: "req-0", pane_id: "p", state: "refused", reason: "asleep" })).toBeNull();
    expect(settleReply(out, { request_id: "req-1", pane_id: "p", state: "sending" })).toBeNull();
    expect(settleReply({ ...out, sent: null }, { request_id: "req-1", pane_id: "p", state: "sent" })).toBeNull();
  });

  it("empties the field once its words were sent, and keeps them with the reason when refused", () => {
    expect(settleReply(out, { request_id: "req-1", pane_id: "p", state: "sent" })).toEqual({ text: "", sent: null, refused: null });
    expect(settleReply(out, { request_id: "req-1", pane_id: "p", state: "refused", reason: "session_changed" })).toEqual({ sent: null, refused: "session_changed" });
    expect(settleReply(out, { request_id: "req-1", pane_id: "p", state: "refused" })).toEqual({ sent: null, refused: "failed" });
  });

  it("reads a reply with no receipt after the wait as unknown, keeping its words", () => {
    expect(settleReply(out, null, true)).toEqual({ sent: null, refused: "unknown" });
  });

  it("finds each request's own receipt in a frame that carries later ones", () => {
    const section = { receipts: [{ request_id: "r1", pane_id: "p", state: "sent" as const }, { request_id: "r2", pane_id: "p", state: "refused" as const, reason: "in_flight" }] };
    expect(replyReceipt(section, "r2")?.reason).toBe("in_flight");
    expect(replyReceipt(section, "r1")?.state).toBe("sent");
    expect(replyReceipt(section, "r3")).toBeNull();
  });
});

describe("reply fields", () => {
  it("keeps at most the cap, dropping the least recently changed field that is not sending", () => {
    const { change } = useReplyFields.getState();
    change("sending", null, { text: "가는 중", sent: "req" });
    for (let index = 0; index < REPLY_FIELDS_MAX + 3; index++) change(`p${index}`, null, { text: `초안 ${index}` });
    const fields = useReplyFields.getState().fields;
    expect(Object.keys(fields)).toHaveLength(REPLY_FIELDS_MAX);
    expect(fields.sending?.sent).toBe("req");
    expect(fields.p0).toBeUndefined();
    expect(fields[`p${REPLY_FIELDS_MAX + 2}`]?.text).toBe(`초안 ${REPLY_FIELDS_MAX + 2}`);
  });

  it("retires the field of a pane no agent runs any more, once the page is live", () => {
    useShellStore.setState({ connection: "live", agents: [legacyAgentRow({ id: "a", pane_id: "a" }), legacyAgentRow({ id: "b", pane_id: "b" })] });
    useReplyFields.getState().change("a", null, { text: "남는 초안" });
    useReplyFields.getState().change("b", null, { text: "닫힌 패널" });
    useShellStore.setState({ agents: [legacyAgentRow({ id: "a", pane_id: "a" })] });
    expect(Object.keys(useReplyFields.getState().fields)).toEqual(["a"]);
    // A reconnect's empty frame keeps the drafts.
    useShellStore.setState({ agents: [] });
    expect(Object.keys(useReplyFields.getState().fields)).toEqual(["a"]);
  });

  it("does not show words written for another session in the same pane", () => {
    useReplyFields.getState().change("a", "session-1", { text: "예전 세션에" });
    useReplyFields.getState().change("a", "session-2", { text: "" });
    expect(useReplyFields.getState().fields.a).toBeUndefined();
  });
});

describe("useAgentReply", () => {
  function mount(send: (pane: string, text: string, session: string | null) => string | null) {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    let reply: ReturnType<typeof useAgentReply> | null = null;
    function Probe() {
      reply = useAgentReply("a", "s", send);
      return null;
    }
    const root = createRoot(document.body.appendChild(document.createElement("div")));
    act(() => root.render(<Probe />));
    return { reply: () => reply!, unmount: () => act(() => root.unmount()) };
  }

  it("clears a refusal once the words are edited", () => {
    const view = mount(() => "req-1");
    act(() => view.reply().setText("배포해"));
    act(() => view.reply().submit());
    act(() => useShellStore.setState({ paneReplies: { receipts: [{ request_id: "req-1", pane_id: "a", state: "refused", reason: "asleep" }] } }));
    expect(view.reply().refused).toBe("asleep");
    expect(view.reply().text).toBe("배포해");
    act(() => view.reply().setText("배포해 주세요"));
    expect(view.reply().refused).toBeNull();
    act(() => view.reply().setText(""));
    expect(useReplyFields.getState().fields.a).toBeUndefined();
    view.unmount();
  });

  it("reads a reply the core never answered as unknown after the wait, and one it answered as it said", () => {
    vi.useFakeTimers();
    useShellStore.setState({ paneReplies: null });
    const view = mount(() => "req-lost");
    act(() => view.reply().setText("계속"));
    act(() => view.reply().submit());
    expect(view.reply().sending).toBe(true);
    act(() => vi.advanceTimersByTime(REPLY_RECEIPT_WAIT_MS - 1));
    expect(view.reply().sending).toBe(true);
    act(() => vi.advanceTimersByTime(1));
    expect(view.reply().sending).toBe(false);
    expect(view.reply().refused).toBe("unknown");
    expect(view.reply().text).toBe("계속");
    view.unmount();
  });

  it("keeps waiting while the core's receipt says sending", () => {
    vi.useFakeTimers();
    const view = mount(() => "req-slow");
    act(() => view.reply().setText("계속"));
    act(() => view.reply().submit());
    act(() => useShellStore.setState({ paneReplies: { receipts: [{ request_id: "req-slow", pane_id: "a", state: "sending" }] } }));
    act(() => vi.advanceTimersByTime(REPLY_RECEIPT_WAIT_MS * 2));
    expect(view.reply().sending).toBe(true);
    view.unmount();
  });
});
