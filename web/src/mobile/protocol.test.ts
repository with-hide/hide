import { describe, expect, it } from "vitest";
import { initializeInterfaceI18n } from "../i18n/instance";
import { MAX_MESSAGES, boxDrawingRow, headerLine, mergeConversation, messageTime, macNameOf, noticeText, notificationRow, openKey, parseFragment, replyProblem, rowsProblem, staleTags, toBase64Url, type AgentGroup, type ConversationMessage, type PhoneAgent } from "./protocol";

const CREDENTIAL = "a".repeat(64);
const korean = initializeInterfaceI18n("ko").getFixedT(null, "translation");

describe("parseFragment", () => {
  it("reads the QR's pairing payload, a credential and a deep link", () => {
    const pair = toBase64Url(JSON.stringify({ v: 1, endpoint: "https://mac.tailnet.ts.net", code: "c0de" }));
    expect(parseFragment(`#pair=${pair}`).pair).toEqual({ v: 1, endpoint: "https://mac.tailnet.ts.net", code: "c0de" });
    expect(parseFragment(`#k=${CREDENTIAL}`).credential).toBe(CREDENTIAL);
    expect(parseFragment(`#open=${encodeURIComponent("local|p1")}`).open).toEqual({ device_id: "local", pane_id: "p1" });
  });

  it("refuses a payload of another version, a malformed credential and a garbled code", () => {
    const v2 = toBase64Url(JSON.stringify({ v: 2, endpoint: "https://x", code: "c" }));
    expect(parseFragment(`#pair=${v2}`).pair).toBeNull();
    expect(parseFragment("#pair=%%%").pair).toBeNull();
    expect(parseFragment("#k=nothex").credential).toBeNull();
  });

  it("keeps a remote pane id whole after the device", () => {
    expect(openKey("dev-1|remote:dev-1:pane:w1-2")).toEqual({ device_id: "dev-1", pane_id: "remote:dev-1:pane:w1-2" });
    expect(openKey("|p")).toBeNull();
  });
});

describe("replyProblem", () => {
  it("matches what hided refuses", () => {
    expect(replyProblem("  ")).toBe("empty");
    expect(replyProblem("x".repeat(2001))).toBe("too_long");
    expect(replyProblem("a\nb")).toBe("control_characters");
    expect(replyProblem("yes, 계속")).toBeNull();
  });
});

function agent(group: AgentGroup["group"], pane: string, root = pane, demand = ""): PhoneAgent {
  return {
    device_id: "local",
    pane_id: pane,
    root_pane_id: root,
    group,
    mark: "turn",
    wait: null,
    descendants_working: 0,
    agent_kind: "claude",
    title: pane,
    place: null,
    device_label: null,
    changed_at_unix_ms: null,
    line: null,
    status_code: "unknown",
    demand,
    emphasized: group === "needs_you" || group === "done",
    // Frozen pre-refactor phone values, including the read-question difference.
    holds_notification: group === "needs_you" || group === "done" || ["question", "approval", "error"].includes(demand),
  };
}

describe("staleTags", () => {
  it("retains a read root question even though its group is Seen", () => {
    expect(staleTags(["local|root"], [{ group: "seen", count: 1, agents: [agent("seen", "root", "root", "question")] }])).toEqual([]);
    expect(staleTags(["local|root"], [{ group: "seen", count: 1, agents: [agent("seen", "root")] }])).toEqual(["local|root"]);
  });

  it("keeps the notification of a root still waiting or done, through a child's request", () => {
    const groups: AgentGroup[] = [
      { group: "needs_you", count: 1, agents: [agent("needs_you", "child", "root")] },
      { group: "seen", count: 1, agents: [agent("seen", "other")] },
    ];
    expect(staleTags(["local|root", "local|other", "local|gone"], groups)).toEqual(["local|other", "local|gone"]);
  });

  it("keeps a root's notification while a delegated child, only ever Working, still asks", () => {
    const groups: AgentGroup[] = [
      { group: "working", count: 1, agents: [agent("working", "child", "root", "question")] },
      { group: "seen", count: 1, agents: [agent("seen", "root")] },
    ];
    expect(staleTags(["local|root"], groups)).toEqual([]);
    expect(staleTags(["local|root"], [{ group: "seen", count: 1, agents: [agent("seen", "root")] }])).toEqual(["local|root"]);
  });
});

describe("notificationRow", () => {
  const base = { pushMode: "always" as const, notifications: "unasked" as const, permission: "default" as const, supported: true };

  it("offers the notification switch only while push is on and the phone has not answered", () => {
    expect(notificationRow(base)).toBe("enable");
    expect(notificationRow({ ...base, pushMode: "off" })).toBeNull();
    expect(notificationRow({ ...base, notifications: "on", permission: "granted" })).toBeNull();
  });

  it("sends a Safari tab to the Home Screen first and names a refused permission", () => {
    expect(notificationRow({ ...base, supported: false, permission: "unsupported" })).toBe("install_first");
    expect(notificationRow({ ...base, permission: "denied" })).toBe("denied");
  });
});

describe("header", () => {
  it("names the Mac from its ts.net address and counts the other phones", () => {
    expect(macNameOf("https://example-mbp.tailnet.ts.net")).toBe("example-mbp");
    expect(headerLine(korean, "mac", 1)).toBe("mac · 폰 1대 더 연결됨");
    expect(headerLine(korean, "mac", 0)).toBe("mac");
    expect(macNameOf("not an address")).toBe("");
  });
});

describe("rowsProblem", () => {
  it("names a closed pane and a device that is not connected (B28)", () => {
    expect(rowsProblem("ok")).toBeNull();
    expect(noticeText(korean, rowsProblem("gone")!)).toBe("이 터미널은 더 이상 열려 있지 않아요.");
    expect(noticeText(korean, rowsProblem("device_unreachable")!)).toBe("이 에이전트의 기기가 연결돼 있지 않아요.");
  });
});

describe("boxDrawingRow", () => {
  it("is a row of box-drawing characters and spaces only", () => {
    expect(boxDrawingRow("─".repeat(200))).toBe(true);
    expect(boxDrawingRow("  ╭────╮  ")).toBe(true);
    expect(boxDrawingRow("│ ❯ ")).toBe(false);
    expect(boxDrawingRow("── 3 files ──")).toBe(false);
    expect(boxDrawingRow("")).toBe(false);
    expect(boxDrawingRow("    ")).toBe(false);
  });
});

describe("mergeConversation", () => {
  const message = (id: number): ConversationMessage => ({ id, who: "agent", text: `m${id}`, truncated: false, at_ms: 0 });
  const ids = (list: ConversationMessage[]) => list.map((item) => item.id);

  it("replaces on a fresh page, prepends an older one, and appends once each", () => {
    const fresh = mergeConversation(null, { mode: "reset", messages: [message(30), message(40)], before: 30 });
    expect(fresh).toEqual({ messages: [message(30), message(40)], before: 30 });
    const older = mergeConversation(fresh, { mode: "older", messages: [message(10), message(20), message(30)], before: null });
    expect(ids(older.messages)).toEqual([10, 20, 30, 40]);
    expect(older.before).toBeNull();
    const appended = mergeConversation(older, { mode: "append", messages: [message(40), message(50)] });
    expect(ids(appended.messages)).toEqual([10, 20, 30, 40, 50]);
    expect(mergeConversation(appended, { mode: "reset", messages: [message(60)], before: 60 })).toEqual({ messages: [message(60)], before: 60 });
  });

  it("drops the oldest past the cap and keeps the page before them readable", () => {
    const full = { messages: Array.from({ length: MAX_MESSAGES }, (_, index) => message(index + 1)), before: null };
    const next = mergeConversation(full, { mode: "append", messages: [message(MAX_MESSAGES + 1)] });
    expect(next.messages).toHaveLength(MAX_MESSAGES);
    expect(next.messages[0]?.id).toBe(2);
    expect(next.before).toBe(2);
  });
});

describe("messageTime", () => {
  it("is the hour and minute today, with the day before today", () => {
    const now = new Date(2026, 8, 29, 15, 0);
    expect(messageTime(new Date(2026, 8, 29, 9, 5).getTime(), now)).toBe("09:05");
    expect(messageTime(new Date(2026, 8, 28, 23, 41).getTime(), now)).toBe("9/28 23:41");
  });
});
