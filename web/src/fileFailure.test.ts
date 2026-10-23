import { describe, expect, it } from "vitest";
import { downloadFailureText, fileFailureText, listingFailure, readFailureText } from "./fileFailure";
import { initializeInterfaceI18n } from "./i18n/instance";

const english = initializeInterfaceI18n("en").getFixedT(null, "translation");
const korean = initializeInterfaceI18n("ko").getFixedT(null, "translation");

describe("a file change or listing the core or a device refused by code", () => {
  it("is worded in the viewer's language, with the limit its sentence names", () => {
    expect(fileFailureText({ code: "host.already_exists" }, korean)).toBe("같은 이름의 항목이 이미 있어요");
    expect(fileFailureText({ code: "explorer.drop_limit", limit: 500 }, english)).toBe("A drop holds at most 500 items; nothing was added");
  });

  it("reads as the general line when this build does not word the code", () => {
    expect(fileFailureText({ code: "host.from_a_newer_helper" }, english)).toBe("The change was not made");
  });

  it("words a device listing by the helper's refusal, else by the link's state", () => {
    const listing = (code: string, reason: string | null) => listingFailure({ device_id: "mini", root_path: "/repo", code, reason });
    expect(fileFailureText(listing("refused", "permission_denied"), english)).toBe("Permission denied");
    expect(fileFailureText(listing("not_ready", null), english)).toBe("The device is not connected");
    expect(fileFailureText(listing("unknown", null), korean)).toBe("기기가 제때 답하지 않았어요");
  });

  it("words a read and a download by their code, and an unknown one says only that it was not read", () => {
    expect(readFailureText("changes.git_failed", korean)).toBe("Git이 이 저장소를 읽지 못했어요");
    expect(readFailureText("host.from_a_newer_helper", english)).toBe("Hide could not read it");
    expect(downloadFailureText("too_large", english)).toBe("The file is too large to download");
    expect(downloadFailureText("outside_checkout", english)).toBe("The path is outside this checkout");
  });
});
