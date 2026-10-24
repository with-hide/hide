// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { CHAPTERS } from "./chapters";
import { FIRST_CHAPTER, openPractice, PRACTICE_CHANNEL, practiceAddress, type PracticeMessage } from "./channel";

// How the shell opens the practice window on a chapter (docs/UI_BEHAVIOR.md,
// Practice window): a new window from its address, one already open because
// the shell tells it over the channel, in a browser tab as in the desktop app.
afterEach(() => {
  delete window.hideHost;
  vi.unstubAllGlobals();
});

it("starts a newcomer on chapter 0", () => {
  expect(CHAPTERS.find((chapter) => chapter.id === FIRST_CHAPTER)?.number).toBe(0);
});

it("opens a browser tab on the chapter's address and tells a window already open to play it", async () => {
  const open = vi.spyOn(window, "open").mockReturnValue(null);
  const listener = new BroadcastChannel(PRACTICE_CHANNEL);
  const heard = new Promise<PracticeMessage>((resolve) => { listener.onmessage = (event: MessageEvent<PracticeMessage>) => resolve(event.data); });
  openPractice(FIRST_CHAPTER);
  expect(await heard).toEqual({ kind: "play", chapter: "overview" });
  listener.close();
  expect(open).toHaveBeenCalledWith(practiceAddress("en", "overview"), PRACTICE_CHANNEL);
  expect(practiceAddress("ko", "overview")).toBe("/?practice=1&lang=ko&chapter=overview");
  expect(practiceAddress("ko")).toBe("/?practice=1&lang=ko");
});

it("asks the desktop app for no chapter when none is named, so the window opens on its list", () => {
  const bridge = vi.fn();
  window.hideHost = { kind: "electron", platform: "darwin", openPractice: bridge } as never;
  openPractice();
  expect(bridge).toHaveBeenCalledWith("en", undefined);
  openPractice(FIRST_CHAPTER);
  expect(bridge).toHaveBeenLastCalledWith("en", "overview");
});
