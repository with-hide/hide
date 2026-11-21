// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { installKeyboard } from "./keyboard";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { noteKeyboardOwner, type KeyboardOwner } from "./viewFocus";

vi.mock("./host", () => ({ hostKind: () => "browser", keySystem: () => "pc", hostBridge: () => null, browserBridge: () => null }));
let remove: () => void;
beforeEach(() => {
  useShellStore.setState({ rest: null });
  useUiStore.setState({ screen: { kind: "workspace" }, overviewOpen: false, overlay: "none", escapeLayers: [], cycle: null, workspaceDialog: null, pendingClose: null, pendingTrash: null, recordingShortcut: false });
  remove = installKeyboard(createActions(() => {}));
});
afterEach(() => { remove(); document.body.replaceChildren(); });

it.each<KeyboardOwner>([{ kind: "none" }, { kind: "view", workspace: "c", areaId: "v1" }, { kind: "tool", workspace: "c" }])("keeps text selection keys with the input owner $kind", (owner) => {
  const field = document.createElement("input");
  document.body.append(field);
  field.focus();
  noteKeyboardOwner(owner);
  const event = new KeyboardEvent("keydown", { code: "ArrowLeft", key: "ArrowLeft", ctrlKey: true, shiftKey: true, bubbles: true, cancelable: true });
  field.dispatchEvent(event);
  expect(event.defaultPrevented).toBe(false);
});

it("keeps selection keys in a covering field even if the last owner was a terminal", () => {
  noteKeyboardOwner({ kind: "pane", workspace: "c", paneId: "p1" });
  useUiStore.setState({ overlay: "settings" });
  const event = new KeyboardEvent("keydown", { code: "ArrowRight", key: "ArrowRight", ctrlKey: true, shiftKey: true, bubbles: true, cancelable: true });
  window.dispatchEvent(event);
  expect(event.defaultPrevented).toBe(false);
});
