import { expect, it } from "vitest";
import { historyAnswer, historyInitial, historyPage, HISTORY_PAGE_LIMIT } from "./overviewHistoryState";
import { requestParts } from "./RequestHighlight";
import type { SessionSearchHit, SessionWorkSearch } from "./snapshot";

const hit = (device: string): SessionSearchHit => ({ session: { device_id: device, agent: "codex", session_id: "same-native-id" }, title: device, matched: "needle", at_unix_ms: 100, cwd: null });
const answer = (request: string, results = [hit("device-b")]): SessionWorkSearch => ({ query: "needle", options: { request_id: request }, loading: false, failure: null, results });

it("keeps a history page when a stale query or the command palette answers the shared mailbox", () => {
  const held = { ...historyInitial("device-a"), requestId: "history-2", query: "needle", results: [hit("device-a")], scroll: 340, selected: "device-a\0codex\0same-native-id" };
  expect(historyAnswer(held, answer("history-1"))).toBe(held);
  expect(historyAnswer(held, { ...answer(""), options: undefined })).toBe(held);
  const next = historyAnswer(held, answer("history-2"));
  expect(next.results[0]?.session.device_id).toBe("device-b");
  expect([next.scroll, next.selected]).toEqual([340, held.selected]);
  expect(next.loading).toBe(false);
});

it("preserves the full tuple for next and previous pages without retaining more than one page of rows", () => {
  const cursor = { at_unix_ms: 100, device_id: "remote", agent: "claude", session_id: "same-native-id" };
  const held = { ...historyInitial("scope"), loading: false, next: cursor, results: [hit("device-a")], scroll: 200 };
  const next = historyPage(held, 1);
  expect(next.cursors).toEqual([null, cursor]);
  expect([next.page, next.scroll, next.results.length]).toEqual([1, 0, 0]);
  const previous = historyPage({ ...next, loading: false }, -1);
  expect([previous.page, previous.cursors[previous.page]]).toEqual([0, null]);
  expect(historyPage({ ...held, page: HISTORY_PAGE_LIMIT - 1 }, 1).page).toBe(HISTORY_PAGE_LIMIT - 1);
});

it("highlights literal multilingual request text without treating user input as a regular expression", () => {
  expect(requestParts("한글 (a+b) 한글", "(a+b)").filter((part) => part.match).map((part) => part.text)).toEqual(["(a+b)"]);
  expect(requestParts("Fix FIX 한글", "fix").filter((part) => part.match).map((part) => part.text)).toEqual(["Fix", "FIX"]);
  expect(requestParts("이 요청을 검색", "요청").map((part) => part.text).join("")).toBe("이 요청을 검색");
  expect(requestParts("no matches", "[" )).toEqual([{ text: "no matches", match: false }]);
  expect(requestParts("full text", "  ")).toEqual([{ text: "full text", match: false }]);
});
