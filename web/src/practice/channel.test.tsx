// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot } from "react-dom/client";
import type { Actions } from "../actions";
import { clientI18n } from "../i18n/translator";
import { useStartPanel } from "../startDraft";
import { useShellStore } from "../store";
import { CHAPTERS } from "./chapters";
import { FIRST_CHAPTER, openPractice, PRACTICE_CHANNEL, practiceAddress, usePracticeChannel, type PracticeMessage } from "./channel";

// How the shell opens the practice window on a chapter (docs/UI_BEHAVIOR.md,
// Practice window): a new window from its address, one already open because
// the shell tells it over the channel, in a browser tab as in the desktop app.
afterEach(() => {
  delete window.hideHost;
  vi.unstubAllGlobals();
});

it("starts a newcomer on the first flow of the list", () => {
  expect(CHAPTERS[0]!.id).toBe(FIRST_CHAPTER);
});

/** The shell hearing the practice window's last card: what it did, from the first thing it did. */
async function missionHeard(workspaces: unknown[]) {
  const actions = { openStartPanel: vi.fn(), openAddProject: vi.fn() } as unknown as Actions;
  useShellStore.setState({ rest: { navigator: { workspaces } } as never });
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const Listening = () => { usePracticeChannel(actions); return null; };
  const root = createRoot(document.createElement("div"));
  await act(async () => root.render(<Listening />));
  const sender = new BroadcastChannel(PRACTICE_CHANNEL);
  sender.postMessage({ kind: "mission" } satisfies PracticeMessage);
  await vi.waitFor(() => expect(vi.mocked(actions.openStartPanel).mock.calls.length + vi.mocked(actions.openAddProject).mock.calls.length).toBe(1));
  sender.close();
  await act(async () => root.unmount());
  return actions;
}

it("opens the Start panel on a mission sentence with its blank selected, unsent", async () => {
  const actions = await missionHeard([{ id: "w1" }]);
  const sentence = clientI18n.t("startPanel.missionSentence");
  const at = sentence.indexOf("___");
  expect(actions.openStartPanel).toHaveBeenCalledTimes(1);
  expect(useStartPanel.getState().text).toBe(sentence);
  expect(useStartPanel.getState().blank).toEqual([at, at + 3]);
  expect(useStartPanel.getState().request).toBeNull();
});

it("asks for a project first when there is none, keeping the sentence as the draft", async () => {
  const actions = await missionHeard([]);
  expect(actions.openAddProject).toHaveBeenCalledTimes(1);
  expect(actions.openStartPanel).not.toHaveBeenCalled();
  expect(useStartPanel.getState().text).toBe(clientI18n.t("startPanel.missionSentence"));
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
