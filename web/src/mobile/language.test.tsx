// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { clientI18n, translate } from "../i18n/translator";
import { PhoneLanguageBoundary } from "./language";
import type { ServerFrame } from "./protocol";
import { applyFrame, patch, usePhone } from "./store";

let root: Root;
let browserLanguage = "en-US";
const postMessage = vi.fn();

async function mount(): Promise<void> {
  await act(async () => root.render(<PhoneLanguageBoundary />));
}

async function choose(preference: string | null): Promise<void> {
  await act(async () => patch({ interfaceLanguage: preference }));
}

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  browserLanguage = "en-US";
  vi.spyOn(window.navigator, "language", "get").mockImplementation(() => browserLanguage);
  postMessage.mockReset();
  Object.defineProperty(navigator, "serviceWorker", { configurable: true, value: { ready: Promise.resolve({ active: { postMessage } }) } });
  root = createRoot(document.createElement("div"));
});

afterEach(async () => {
  await act(async () => root.unmount());
  Reflect.deleteProperty(navigator, "serviceWorker");
  patch({ interfaceLanguage: null });
  vi.restoreAllMocks();
  await clientI18n.changeLanguage("en");
});

describe("the phone's language", () => {
  it("follows the daemon's explicit choice over the phone's own language", async () => {
    browserLanguage = "ja-JP";
    await choose("ko");
    await mount();
    expect(translate("mobile.group.done")).toBe("읽을 것");
    expect(document.documentElement.lang).toBe("ko");
  });

  it("follows the phone's language while no choice exists, and again when it changes", async () => {
    browserLanguage = "ja-JP";
    await mount();
    expect(translate("mobile.group.done")).toBe("未読");
    browserLanguage = "zh-Hans-CN";
    await act(async () => window.dispatchEvent(new Event("languagechange")));
    expect(translate("mobile.group.done")).toBe("待读");
    expect(document.documentElement.lang).toBe("zh-CN");
  });

  it("shows English for an unsupported phone language and for an unknown stored choice", async () => {
    browserLanguage = "fr-FR";
    await mount();
    expect(translate("mobile.group.done")).toBe("Unread");
    browserLanguage = "ko-KR";
    await choose("fr");
    expect(translate("mobile.group.done")).toBe("Unread");
  });

  it("keeps the language and color theme the last agents frame carried", () => {
    const frame = { type: "agents", groups: [], interface_language: "zh-CN", color_theme: "neutral" } satisfies ServerFrame;
    applyFrame(frame);
    expect(usePhone.getState().interfaceLanguage).toBe("zh-CN");
    expect(usePhone.getState().colorTheme).toBe("neutral");
    applyFrame({ ...frame, interface_language: null, color_theme: "hide" });
    expect(usePhone.getState().interfaceLanguage).toBeNull();
    expect(usePhone.getState().colorTheme).toBe("hide");
  });

  it("gives the service worker the notification words on load and on each language change", async () => {
    await choose("ko");
    await mount();
    await vi.waitFor(() => expect(postMessage.mock.calls.at(-1)?.[0]).toMatchObject({ type: "words", words: { needs_you: "내 차례", done: "읽을 것", observer_unconfirmed: "관찰자가 경고를 확인하지 않았어요", letter_undelivered: "편지가 전달되지 않았어요", status_question: "질문", status_approval: "승인", status_error: "막힘", status_done: "끝남", subject_plan_review: "계획 검토 · {{count}}단계" } }));
    await choose("ja");
    await vi.waitFor(() => expect(postMessage.mock.calls.at(-1)?.[0]).toMatchObject({ type: "words", words: { needs_you: "自分の番", done: "未読", observer_unconfirmed: "オブザーバーが警告を確認していません", letter_undelivered: "レターが届いていません", status_question: "質問", status_approval: "承認", status_error: "ブロック", status_done: "完了", subject_plan_review: "計画レビュー · {{count}}ステップ" } }));
    await choose(null);
    await vi.waitFor(() => expect(postMessage.mock.calls.at(-1)?.[0]).toMatchObject({ type: "words", words: { needs_you: "Needs You", done: "Unread", observer_unconfirmed: "Observer has not confirmed a warning", letter_undelivered: "Letter not delivered", status_question: "Question", status_approval: "Approval", status_error: "Blocked", status_done: "Done", subject_plan_review: "Plan review · {{count}} steps" } }));
  });
});
