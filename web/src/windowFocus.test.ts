// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";

// The window holds the keyboard throughout; what it draws is what moves.
vi.mock("./viewFocus", () => ({ windowIsActive: () => true, subscribeKeyboardOwner: () => () => {} }));

import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { installWindowFocus, type FocusScreen } from "./windowFocus";

afterEach(() => useUiStore.setState({ remoteScreen: null }));

// PRD unseen-agent-views D-07: a View is seen only in front of a window that draws it. Another computer's screen in
// this window draws nothing of this machine's, so the window names no screen while it shows one.
it("names no screen of this machine while the window shows another computer's", () => {
  useShellStore.setState({ connection: "live" } as never);
  useUiStore.setState({ screen: { kind: "workspace" }, remoteScreen: null } as never);
  const sent: FocusScreen[] = [];
  const remove = installWindowFocus((screen) => sent.push(screen));
  useUiStore.setState({ remoteScreen: "mini" });
  useUiStore.setState({ remoteScreen: null });
  remove();
  expect(sent).toEqual(["workspace", null, "workspace"]);
});
