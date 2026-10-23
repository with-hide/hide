import { legacyAgentRow } from "../test/legacyAgentRow";
import { beforeAll, describe, expect, it } from "vitest";
import type { TFunction } from "i18next";
import { createInterfaceI18n } from "./i18n/instance";
import { paneMenuItems } from "./PaneRelations";
import { sleepAfterChoice, sleepingCount } from "./settings";
import { sleepCaption, wakeFailureReason, wakingLine } from "./sleep";
import type { AgentRow, PaneRow } from "./snapshot";

const HOUR = 60 * 60 * 1000;
const now = 100 * HOUR;

let t: TFunction<"translation">;
let ko: TFunction<"translation">;
beforeAll(async () => {
  t = (await createInterfaceI18n("en")).t;
  ko = (await createInterfaceI18n("ko")).t;
});

describe("a sleeping pane's words", () => {
  it("says how long the agent has slept, then waking, then that it could not resume", () => {
    expect(sleepCaption({ state: "sleeping", since_unix_ms: now - 22 * HOUR }, now, t)).toEqual({ text: "sleeping · 22h", moon: true });
    expect(sleepCaption({ state: "sleeping", since_unix_ms: now - 1000 }, now, t)).toEqual({ text: "sleeping", moon: true });
    expect(sleepCaption({ state: "waking", since_unix_ms: now - 22 * HOUR }, now, t)).toEqual({ text: "waking…", moon: true });
    expect(sleepCaption({ state: "failed", since_unix_ms: now, reason: "The conversation couldn’t be resumed." }, now, t)).toEqual({
      text: "could not resume",
      moon: false,
    });
  });

  it("names the age of the conversation a wake resumes", () => {
    expect(wakingLine({ state: "waking", since_unix_ms: now - 3 * 24 * HOUR }, now, t)).toBe("Resuming the conversation from 3d ago");
    expect(wakingLine({ state: "waking", since_unix_ms: now }, now, t)).toBe("Resuming the conversation");
  });

  it("words a wake the core refused for an unread Codex in the chosen language, and keeps any other reason as sent", () => {
    const refused = { state: "failed", since_unix_ms: now, reason: "codex_capability_unknown" } as const;
    expect(wakeFailureReason(refused, t)).toBe("Hide hasn't read this machine's Codex yet. Check Codex in Settings > Agents, then try again.");
    expect(wakeFailureReason(refused, ko)).toBe("이 기기의 Codex를 아직 확인하지 못했어요. 설정 > 에이전트에서 Codex를 확인한 뒤 다시 시도하세요.");
    expect(wakeFailureReason({ state: "failed", since_unix_ms: now, reason: "The working folder /w no longer exists." }, t)).toBe("The working folder /w no longer exists.");
    expect(wakeFailureReason({ state: "failed", since_unix_ms: now }, t)).toBeNull();
  });

  it("words the caption and the age line in the chosen language", () => {
    expect(sleepCaption({ state: "sleeping", since_unix_ms: now - 22 * HOUR }, now, ko)).toEqual({ text: "재움 · 22시간", moon: true });
    expect(wakingLine({ state: "waking", since_unix_ms: now - 3 * 24 * HOUR }, now, ko)).toBe("3일 전 대화를 다시 시작하는 중");
  });

  it("leaves the age out under a minute in every language, not by comparing the word", () => {
    expect(sleepCaption({ state: "sleeping", since_unix_ms: now - 1000 }, now, ko)).toEqual({ text: "재움", moon: true });
    expect(wakingLine({ state: "waking", since_unix_ms: now }, now, ko)).toBe("대화를 다시 시작하는 중");
  });
});

describe("the Sleep idle agents setting and the pane menu", () => {
  it("reads a stored choice and reads anything it does not offer as Never", () => {
    expect([null, undefined, 12, 24, 72, 5, "24"].map(sleepAfterChoice)).toEqual(["never", "never", "12", "24", "72", "never", "never"]);
  });

  it("counts only this machine's sleeping agents", () => {
    const row = (pane_id: string, sleeping: boolean) =>
      (legacyAgentRow({ pane_id, ...(sleeping ? { sleep: { state: "sleeping", since_unix_ms: 0 } } : {}) })) as AgentRow;
    expect(sleepingCount([row("w1:p1", true), row("w1:p2", false), row("remote:mini:pane:w1:p1", true)])).toBe(1);
  });

  it("offers Sleep agent on an awake agent pane, disabled with the core's reason when it cannot sleep", () => {
    const pane = (sleep_action?: PaneRow["sleep_action"]) =>
      ({ id: "w1:p1", sleep_action, children: null, lineage_path: [] }) as unknown as PaneRow;
    const sleepItem = (row: PaneRow) => paneMenuItems(row, "reviewer").find((item) => item.id === "sleep_agent");
    expect(sleepItem(pane({ available: true }))).toMatchObject({ label: "Sleep agent", unavailable: null });
    expect(sleepItem(pane({ available: false, reason: "working" }))).toMatchObject({ unavailable: "This agent is working" });
    expect(sleepItem(pane({ available: false, reason: "a_newer_reason" as never }))).toMatchObject({ unavailable: "This agent cannot sleep now" });
    expect(sleepItem(pane())).toBeUndefined();
  });
});
