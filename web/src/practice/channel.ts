// What the practice window and the shell say to each other. The practice page
// holds no daemon connection: it asks the shell of the same origin, over one
// BroadcastChannel, for the operator's language, chords and progress, and
// hands each step it reaches back for the shell to store in the core
// (`practice_progress_set`). This module is in the shell's bundle, so it
// imports nothing of the practice chapters.

import { useEffect } from "react";
import type { Actions } from "../actions";
import { hostBridge } from "../host";
import { clientI18n } from "../i18n/translator";
import type { SnapshotRest } from "../snapshot";
import { useShellStore } from "../store";

export const PRACTICE_CHANNEL = "hide-practice";

/** One chapter's progress as the core keeps it: the version played, the step reached, whether the end was reached. */
export type ChapterProgress = { version: number; step: number; finished: boolean };
export type PracticeProgress = Record<string, ChapterProgress>;

type UiState = NonNullable<SnapshotRest["ui_state"]>;

/** What the shell answers: everything the practice page needs from the operator's Hide. */
export type ShellAnswer = {
  kind: "state";
  progress: PracticeProgress;
  language: string;
  shortcut_bindings: UiState["shortcut_bindings"];
  browser_shortcut_bindings: UiState["browser_shortcut_bindings"];
  theme: UiState["theme"];
  color_theme: UiState["color_theme"];
};

export type PracticeMessage = { kind: "hello" } | ({ kind: "progress"; chapter: string } & ChapterProgress);

/** The core fields an answer carries; a snapshot that moves none of them tells the practice window nothing. */
const ANSWERED = ["practice", "interface_language", "shortcut_bindings", "browser_shortcut_bindings", "theme", "color_theme"] as const;

function answer(): ShellAnswer {
  const ui = useShellStore.getState().rest?.ui_state;
  return {
    kind: "state",
    progress: ui?.practice ?? {},
    language: clientI18n.language,
    shortcut_bindings: ui?.shortcut_bindings,
    browser_shortcut_bindings: ui?.browser_shortcut_bindings,
    theme: ui?.theme,
    color_theme: ui?.color_theme,
  };
}

/** The shell's side: answers a practice window's hello, tells it of every change it cares about, and stores the steps it reports. */
export function usePracticeChannel(actions: Actions): void {
  useEffect(() => {
    const channel = new BroadcastChannel(PRACTICE_CHANNEL);
    channel.onmessage = (event: MessageEvent<PracticeMessage>) => {
      const message = event.data;
      if (message.kind === "hello") channel.postMessage(answer());
      else if (message.kind === "progress") actions.setPracticeProgress(message.chapter, { version: message.version, step: message.step, finished: message.finished });
    };
    const unsubscribe = useShellStore.subscribe((state, previous) => {
      const now = state.rest?.ui_state;
      const before = previous.rest?.ui_state;
      if (ANSWERED.every((field) => now?.[field] === before?.[field])) return;
      channel.postMessage(answer());
    });
    return () => {
      unsubscribe();
      channel.close();
    };
  }, [actions]);
}

/** The practice address on this shell's origin, in the operator's language. */
export function practiceAddress(language: string): string {
  return `/?practice=1&lang=${encodeURIComponent(language)}`;
}

/** Opens the practice window: the desktop app's own window, or a tab of its own in a browser (one per shell, reused). */
export function openPractice(): void {
  const bridge = hostBridge();
  if (bridge) bridge.openPractice(clientI18n.language);
  else window.open(practiceAddress(clientI18n.language), PRACTICE_CHANNEL);
}
