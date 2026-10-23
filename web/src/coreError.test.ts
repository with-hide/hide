import { describe, expect, it } from "vitest";
import { coreErrorText, herdrStateText, remoteStateText } from "./coreError";
import { initializeInterfaceI18n } from "./i18n/instance";

const english = initializeInterfaceI18n("en").getFixedT(null, "translation");
const korean = initializeInterfaceI18n("ko").getFixedT(null, "translation");

describe("a failure the core names by its kind", () => {
  it("is worded in the viewer's language, never with the core's sentence", () => {
    const refused = { kind: "workspace.create_failed", message: "worktree.open was refused: invalid_params: path is not a directory" };
    expect(coreErrorText(refused, english)).toBe("The project could not be added. If it keeps happening, check Recent in Settings.");
    expect(coreErrorText(refused, korean)).toBe("프로젝트를 추가하지 못했어요. 계속 그러면 설정의 최근 기록을 확인하세요.");
  });

  it("is the surface's general sentence when this build does not word the kind, as from a daemon of another build", () => {
    expect(coreErrorText({ kind: "workspace.create_from_the_future" }, english, "workspace.createFailed")).toBe("The worktree was not created.");
    expect(coreErrorText({ kind: "workspace.create_from_the_future" }, korean)).toBe("요청을 처리하지 못했어요. 다시 시도하세요.");
  });
});

describe("a connection state the core names by its code", () => {
  it("words this machine's Herdr and a device without the core's socket paths", () => {
    expect(herdrStateText("socket_missing", korean)).toBe("Herdr가 실행 중이 아니에요.");
    expect(herdrStateText("a_new_state", english)).toBe("Herdr is not connected.");
    expect(remoteStateText("unreachable", "Mac mini", korean)).toBe("Mac mini 기기에 연결하지 못했어요");
    expect(remoteStateText("ssh_auth_failed", "Mac mini", english)).toBe("Mac mini is not connected");
  });
});
