// What a practice chapter plays in: a world the real shell draws, and the core
// events it answers, applied here the way the core would apply them. Nothing
// leaves the page: no daemon, no engine, no agent.

import type { AccessCall } from "../accessCall";
import type { AutomationCall, CallAnswer } from "../automationCall";
import type { NodeUsage } from "../generated/hided-ws";
import type { PhoneAsk } from "../mobile/protocol";
import type { ComposerFile, FileNotice, SentFile } from "../mobile/uploads";
import type { AccessSection, AgentRow, AutomationsSection, ChangesSnapshot, EditorSnapshot, LinkPanel, MemorySection, SessionRecordSection, SessionWorkSearch, SnapshotRest, SuggestionsSection } from "../snapshot";
import type { PairingResult, PairingView } from "../pairing";
import type { DirectoryList, Documents, FileIndexResult } from "../store";

export type CoreEvent = { kind: string; payload: Record<string, unknown> };

/**
 * What the practice window sends a world besides the core's events: text the
 * shell would have put on the clipboard (`{ text, what }`). A paste on a
 * practice screen takes the last of them back; the operator's clipboard is
 * never read or written from a practice screen.
 */
export const PRACTICE_COPY = "practice_copy";

/**
 * What the practice window sends a world when the shell would have left the
 * page (`{ what, detail }`: a window it would open, a link, a navigation, or a
 * call on the desktop host), held there instead (`outside.ts`). A world that
 * has no answer for it keeps it as told.
 */
export const PRACTICE_OUTSIDE = "practice_outside";

/**
 * What the core sends beside the snapshot: the ⌘P index's answer, the open
 * editor tabs and their documents, ⌘K's session search, the record and link
 * record a desk's card tab shows, the Explorer's folder listings (keyed by
 * folder path) and the checkout's Git changes, and the Memory tool's Project
 * with the Hide AI tool's 제안 section it is drawn under, Settings › 봇's
 * requests and connected bots, and hided's `pairing` frame with its answer to
 * the last `pairing_*` press (a world without one has nothing under way).
 */
export type WorldFrames = {
  fileIndex?: FileIndexResult | null; editor?: EditorSnapshot | null; documents?: Documents;
  sessionWorkSearch?: SessionWorkSearch | null; sessionRecord?: SessionRecordSection | null; linkPanel?: LinkPanel | null;
  listings?: Record<string, DirectoryList>; changes?: ChangesSnapshot | null; memory?: MemorySection | null; suggestions?: SuggestionsSection | null;
  access?: AccessSection | null;
  pairing?: PairingView; pairingResult?: PairingResult | null;
};

/** `automations` is the core's section of the automation screen; a world without one sends none, and the shell then holds none. */
export type WorldSnapshot = { rest: SnapshotRest; agents: AgentRow[]; focused: string | null; usage: NodeUsage | null; frames?: WorldFrames; automations?: AutomationsSection };

/**
 * The phone a chapter puts beside the screen (`PracticePhone.tsx`), open on
 * one agent's conversation as the phone draws it: what was said, its ask while
 * it asks, the choice marked on it and whether it was sent, and since when the
 * agent runs a command. Its presses come to the world as `practice_phone_pick`
 * and `practice_phone_answer` (`{ choice }`).
 */
export type PracticePhoneScreen = {
  pane: string;
  title: string;
  kind: string;
  said: { who: "you" | "agent"; text: string }[];
  ask: PhoneAsk | null;
  picked: string | null;
  answered: boolean;
  running: number | null;
  /**
   * The reply bar, for a chapter about a reply's files (Doop 08): the files on it, the line under them, its words, and whether it
   * may be sent. Its `+` comes to the world as `practice_phone_attach`, a file's `×` as `practice_phone_remove` (`{ id }`) and the
   * send as `practice_phone_send`. A world without one draws no bar.
   */
  reply?: { text: string; files: ComposerFile[]; notice: FileNotice | null; sendable: boolean };
  /** The files over an operator's message, by its place in `said` (Doop 09). */
  attached?: Record<number, SentFile[]>;
};

export type PracticeWorld<M> = {
  initial: () => M;
  /** The model after `event`, or null for an event the chapter never needs. */
  apply: (model: M, event: CoreEvent) => M | null;
  snapshot: (model: M, nowMs: number) => WorldSnapshot;
  /** The panes whose terminals show a screen, and that screen for a terminal `cols` wide. */
  panes: (model: M) => string[];
  screen: (model: M, paneId: string, cols: number) => string;
  /**
   * The words typed into `paneId`'s prompt and not sent yet. A world whose
   * chapter types to an agent keeps it, so ⏎ sends what the learner wrote
   * there and types the step's example only into an empty prompt.
   */
  prompt?: (model: M, paneId: string) => string;
  /**
   * What on the screen is an invented record that could pass for a real one
   * (a pull request's title): the coach marks each match `예시`, so the
   * shell's own components stay as they are.
   */
  examples?: string[];
  /**
   * What the world does next on its own, as an agent at work would: the event
   * applied at `at` (epoch ms) unless another event changes the model first,
   * which asks again. A world that only answers the learner has none.
   */
  later?: (model: M) => { at: number; event: CoreEvent } | null;
  /**
   * A call the window waits for an answer to (`automation_call`: the form's
   * preview, make and edit, a run's reopen): the model it leaves, null when it
   * changes none, and the answer the core would send back. A world with no
   * answer refuses every call as the core does with automations unavailable.
   */
  call?: (model: M, call: AutomationCall) => { model: M | null; answer: CallAnswer<unknown> };
  /**
   * Settings › 봇's call (`access_call`: make a bot, give it a new code, read
   * its code again), answered as `call` is. A world with no answer refuses
   * every one as the core does with bots unavailable.
   */
  access?: (model: M, call: AccessCall) => { model: M | null; answer: CallAnswer<unknown> };
  /** The phone beside the screen, for a chapter about the phone; a world without one draws none. */
  phone?: (model: M) => PracticePhoneScreen;
};
