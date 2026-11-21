// The notices the corner holds (docs/UI_BEHAVIOR.md, Notices): at most three,
// the oldest info notice leaving first, and a repeated notice replacing itself.
import { beforeEach, expect, it } from "vitest";
import { useUiStore, type Notice } from "./ui";

const info = (text: string): Notice => ({ level: "info", text, refreshable: false });
const important = (text: string): Notice => ({ level: "important", text, refreshable: false });
const texts = () => useUiStore.getState().notices.map((notice) => notice.text);

beforeEach(() => useUiStore.setState({ notices: [] }));

it("keeps three, dropping the oldest info notice before any important one", () => {
  const { showNotice } = useUiStore.getState();
  showNotice(important("unsaved"));
  showNotice(info("not opened"));
  showNotice(important("status unknown"));
  showNotice(info("not connected"));
  expect(texts()).toEqual(["unsaved", "status unknown", "not connected"]);
});

it("drops the oldest important notice when every older one is important", () => {
  const { showNotice } = useUiStore.getState();
  showNotice(important("a"));
  showNotice(important("b"));
  showNotice(important("c"));
  showNotice(info("d"));
  expect(texts()).toEqual(["b", "c", "d"]);
});

it("replaces a notice with the same key, or the same text when it has none, and shows it newest", () => {
  const { showNotice } = useUiStore.getState();
  showNotice({ ...important("1 agent waiting"), key: "capacity" });
  showNotice(info("not opened"));
  const first = useUiStore.getState().notices.find((notice) => notice.text === "not opened")!.id;
  showNotice({ ...important("2 agents waiting"), key: "capacity" });
  showNotice(info("not opened"));
  expect(texts()).toEqual(["2 agents waiting", "not opened"]);
  // A new id draws it as a new notice, so an info one counts its whole time again.
  expect(useUiStore.getState().notices[1]?.id).not.toBe(first);
});

it("dismisses one notice, the ones a rule names, or all", () => {
  const { showNotice } = useUiStore.getState();
  showNotice(info("a"));
  showNotice({ ...important("b"), refreshable: true });
  showNotice(info("c"));
  useUiStore.getState().dismissNotice(useUiStore.getState().notices[0]!.id);
  expect(texts()).toEqual(["b", "c"]);
  useUiStore.getState().dismissNotices((notice) => notice.refreshable);
  expect(texts()).toEqual(["c"]);
  useUiStore.getState().dismissNotices();
  expect(texts()).toEqual([]);
});
