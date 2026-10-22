import { describe, expect, it } from "vitest";
import { createInterfaceI18n } from "./i18n/instance";
import { commandTitle } from "./commandTitle";
import { agentPeekKeyOn, bindingProblemText, passthroughText, sheetRowTitle } from "./shortcutLabels";
import { REGISTRY, sheetRows } from "./shortcuts";

describe("command labels", () => {
  it("English titles are the registry titles the desktop menu prints", async () => {
    const { t } = await createInterfaceI18n("en");
    for (const command of REGISTRY) expect(commandTitle(command.id, t)).toBe(command.title);
  });

  it("reads a numbered command and a folded numbered row in Korean", async () => {
    const { t } = await createInterfaceI18n("ko");
    expect(commandTitle("select_agent_3", t)).toBe("3번 에이전트 선택");
    const row = sheetRows("Navigate", REGISTRY, "electron", "mac").find((entry) => entry.id === "select_device_1")!;
    expect(sheetRowTitle(row, t)).toBe("1-9번 기기 선택");
    expect(passthroughText("explorer", t)).toBe("파일 탐색기에서만 사용");
  });
});

describe("binding problems", () => {
  it("name the conflicting command in the operator's language", async () => {
    const ko = (await createInterfaceI18n("ko")).t;
    const ja = (await createInterfaceI18n("ja")).t;
    const problem = { code: "conflict", chord: "⌘3", commandId: "select_device_3" } as const;
    expect(bindingProblemText(problem, ko)).toBe("⌘3는 이미 3번 기기 선택에 사용 중입니다.");
    expect(bindingProblemText(problem, ja)).toBe("⌘3はすでに「デバイス3を選択」に割り当てられています。");
  });

  it("keeps the English wording of every code", async () => {
    const { t } = await createInterfaceI18n("en");
    expect(bindingProblemText({ code: "modifier", modifier: "desktop_pc" }, t)).toBe("Include Ctrl+Shift or Alt+Shift so typing in a terminal stays typing.");
    expect(bindingProblemText({ code: "modifier", modifier: "browser_mac" }, t)).toBe("Include ⌘, ⌥ or ⌃ so typing in a terminal stays typing.");
    expect(bindingProblemText({ code: "browser_reserved", chord: "⌘W", macos: true }, t)).toBe("⌘W is kept by Chrome or macOS.");
    expect(bindingProblemText({ code: "browser_reserved", chord: "Ctrl+T", macos: false }, t)).toBe("Ctrl+T is kept by Chrome or the system.");
    expect(bindingProblemText({ code: "terminal_paste", chord: "Ctrl+Shift+V" }, t)).toBe("Ctrl+Shift+V pastes into a terminal.");
  });
});

describe("the key that peeks at the agent logos", () => {
  it("is the Agents hold's own modifier on this system, and absent where the host has none", () => {
    expect(agentPeekKeyOn(undefined, "electron", "mac")).toBe("⌥");
    expect(agentPeekKeyOn(undefined, "electron", "pc")).toBe("Alt");
    expect(agentPeekKeyOn(undefined, "browser", "mac")).toBeNull();
    expect(agentPeekKeyOn(undefined, "browser", "pc")).toBeNull();
  });
});
