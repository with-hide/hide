import { describe, expect, it } from "vitest";
import { initializeInterfaceI18n } from "../i18n/instance";
import { GROUP_TITLE, QUICK_KEYS, REFUSAL_NOTICE, inputFailure, noticeText, type Refusal } from "./protocol";
import { startFailure } from "./start";

const korean = initializeInterfaceI18n("ko").getFixedT(null, "translation");
const english = initializeInterfaceI18n("en").getFixedT(null, "translation");
const japanese = initializeInterfaceI18n("ja").getFixedT(null, "translation");

describe("the phone's key tables", () => {
  it("names the five groups in the shipped Korean and in English", () => {
    expect(Object.values(GROUP_TITLE).map(korean)).toEqual(["내 차례", "읽을 것", "다시 볼 것", "작업 중", "확인함"]);
    expect(Object.values(GROUP_TITLE).map(english)).toEqual(["Needs You", "Unread", "Revisit", "Working", "Seen"]);
  });

  it("names the quick keys by what they press", () => {
    expect(QUICK_KEYS.map((quick) => korean(quick.name))).toEqual(["Enter", "Escape", "위 화살표", "아래 화살표", "Ctrl-C"]);
    expect(QUICK_KEYS.map((quick) => english(quick.name))).toEqual(["Enter", "Escape", "Up arrow", "Down arrow", "Ctrl-C"]);
  });

  it("carries the reply and start limits as formatted numbers", () => {
    expect(noticeText(korean, inputFailure("too_long"))).toBe("답장은 한 번에 2,000자까지 보낼 수 있어요.");
    expect(noticeText(japanese, inputFailure("too_long"))).toBe("返信は一度に2,000文字まで送信できます。");
    expect(noticeText(english, inputFailure("too_long"))).toBe("Replies can contain up to 2,000 characters at a time.");
    expect(noticeText(korean, startFailure("too_long"))).toBe("할 일은 4,000자까지 적을 수 있어요.");
    expect(noticeText(english, startFailure("too_long"))).toBe("Tasks can contain up to 4,000 characters.");
  });

  it("states the phone limit the Mac enforces and shows one line for each refusal", () => {
    expect(noticeText(korean, REFUSAL_NOTICE.phone_limit)).toBe("폰은 4대까지 연결할 수 있어요. 맥의 설정 > 모바일에서 하나를 해지하세요.");
    const refusals: Refusal[] = ["code_expired", "phone_limit", "revoked", "mobile_off", "no_credential"];
    expect(refusals.map((refusal) => noticeText(english, REFUSAL_NOTICE[refusal]))).toEqual([
      "The code expired. Reopen the QR code on your Mac.",
      "You can connect up to 4 phones. Revoke one in Settings > Mobile on your Mac.",
      "This phone's connection was revoked. Reopen the QR code on your Mac.",
      "Not connected · hide on your Mac or Tailscale on your phone is off. Retrying",
      "Scan the QR code in Settings > Mobile on your Mac.",
    ]);
  });

  it("answers an unknown reason with the generic failure, never a raw code", () => {
    expect(noticeText(korean, inputFailure("something.new"))).toBe("보내지 못했어요. 다시 보내세요.");
    expect(noticeText(english, inputFailure(null))).toBe("Couldn't send your reply. Try again.");
    expect(noticeText(english, startFailure("something.new"))).toBe("Couldn't start the agent. Try again.");
  });
});
