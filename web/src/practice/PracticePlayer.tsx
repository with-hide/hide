// One chapter playing: the production shell over the chapter's world, with
// the practice layer on top. The shell's own keyboard listener runs here, and
// every event it dispatches goes to the world instead of a daemon, so a chord
// or a click changes the screen the way the core would and touches nothing
// outside the page.

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createActions } from "../actions";
import { AutomationScreen } from "../AutomationScreen";
import { configureAccessCalls, receiveAccessResult, type AccessCall } from "../accessCall";
import { configureAutomationCalls, receiveAutomationResult, type AutomationCall } from "../automationCall";
import { TooltipProvider } from "../components/ui/tooltip";
import { installKeyboard } from "../keyboard";
import { ConfirmClose, CycleOverlay } from "../Overlays";
import { OverviewModal } from "../Overview";
import { NOTHING_UNDER_WAY } from "../pairing";
import { Palette } from "../Palette";
import { PairingDialogs } from "../PairingDialogs";
import { SettingsGate } from "../SettingsSheet";
import type { SnapshotRest } from "../snapshot";
import { Sidebar } from "../sidebar";
import { useShellStore, type DirectoryList, type Documents } from "../store";
import { feedChunks, terminalFor } from "../terminals";
import { TitleRow } from "../TitleRow";
import { AUTOMATION_ENTRY, useUiStore } from "../ui";
import { MissionScreen } from "../MissionScreen";
import { WorkspaceScreen } from "../WorkspaceScreen";
import type { PracticeChapter } from "./chapter";
import { PracticeCoach } from "./PracticeCoach";
import { PracticePhone } from "./PracticePhone";
import { holdPageClipboard, practiceClipboard } from "./clipboard";
import { holdPageOutside } from "./outside";
import type { CoreEvent } from "./world";

type UiState = NonNullable<SnapshotRest["ui_state"]>;

const NO_DOCUMENTS: Documents = {};
const NO_LISTINGS: Record<string, DirectoryList> = {};

/** Home, then erase the screen and its scrollback: a screen the world changed is drawn from the top. */
const CLEAR = "\x1b[H\x1b[2J\x1b[3J";

/** The operator's own choices the practice screen keeps: their chords, so the keys taught are the keys they press. */
export type OperatorChoices = Pick<UiState, "shortcut_bindings" | "browser_shortcut_bindings">;

type Props = {
  chapter: PracticeChapter;
  next: PracticeChapter | null;
  /** The step the learner left the chapter at; the coach plays the steps before it again on its own. */
  resume: number;
  operator: OperatorChoices;
  onStep: (step: number, finished: boolean) => void;
  /** Starts `chapter` over, the coach playing its steps before `resume` again on its own. */
  onStart: (chapter: PracticeChapter, resume: number) => void;
  onHub: () => void;
  /** Leaves the practice for the real Hide. */
  onLeave: () => void;
};

export function PracticePlayer({ chapter, next, resume, operator, onStep, onStart, onHub, onLeave }: Props) {
  const [model, setModel] = useState<unknown>(() => chapter.world.initial());
  // How many events the world has answered: the signal a step that takes several presses moves on.
  const [moves, setMoves] = useState(0);
  // The sidebar's 미션 row shows the mission screen in the Workspace's place, as in the shell (PRD mission-screen B2).
  const missionsShown = useUiStore((s) => s.screen?.kind === "missions");
  // Likewise the sidebar's 자동화 row, in a world that sends the section; in any other the row leaves the center as it was.
  const automationsRow = useUiStore((s) => s.screen?.kind === "automations");
  const automationsShown = useShellStore((s) => automationsRow && s.automations !== null);
  const live = useRef({ world: chapter.world, model });
  live.current = { world: chapter.world, model };

  const { actions, clipboard, send, answer, answerAccess } = useMemo(() => {
    const commit = (after: unknown) => {
      live.current = { ...live.current, model: after };
      setModel(after);
      setMoves((count) => count + 1);
    };
    const send = (event: CoreEvent) => {
      const after = live.current.world.apply(live.current.model, event);
      if (after) commit(after);
      return true;
    };
    // A call the window waits on (the automation form's preview and make) is answered by the world, to this window alone.
    const answer = (request_id: string, call: AutomationCall) => {
      const { world, model: held } = live.current;
      const result = world.call?.(held, call) ?? { model: null, answer: { ok: false as const, code: "automations_unavailable" } };
      if (result.model) commit(result.model);
      queueMicrotask(() => receiveAutomationResult(result.answer.ok ? { request_id, ok: true, value: result.answer.value } : { request_id, ok: false, code: result.answer.code }));
    };
    // Settings › 봇's calls are answered the same way, a made bot's code invented by the world.
    const answerAccess = (request_id: string, call: AccessCall) => {
      const { world, model: held } = live.current;
      const result = world.access?.(held, call) ?? { model: null, answer: { ok: false as const, code: "access_unavailable" } };
      if (result.model) commit(result.model);
      queueMicrotask(() => receiveAccessResult(result.answer.ok ? { request_id, ok: true, value: result.answer.value } : { request_id, ok: false, code: result.answer.code }));
    };
    // A copy goes to the world and a paste takes the last copy back; the operator's clipboard is never read or written.
    const clipboard = practiceClipboard(send);
    return { actions: createActions(send, clipboard), clipboard, send, answer, answerAccess };
  }, []);

  // The world's own next beat, an agent finishing a turn, plays at its time; any change to the model before then asks again.
  useEffect(() => {
    const beat = chapter.world.later?.(model);
    if (!beat) return undefined;
    const timer = window.setTimeout(() => send(beat.event), Math.max(0, beat.at - Date.now()));
    return () => window.clearTimeout(timer);
  }, [chapter, model, send]);

  useEffect(() => installKeyboard(actions), [actions]);
  useEffect(() => holdPageClipboard(clipboard), [clipboard]);
  // A link, a window, a navigation or a host call the shell would have sent off the page goes to the world instead.
  useEffect(() => holdPageOutside(send), [send]);
  // The automation form's calls go to the world, never to a daemon; what held them before is put back as the window closes.
  useLayoutEffect(() => {
    const before = configureAutomationCalls((event) => {
      if (event.kind !== "automation_call") return false;
      const { request_id, call } = event.payload as { request_id: string; call: AutomationCall };
      answer(request_id, call);
      return true;
    });
    return () => void configureAutomationCalls(before);
  }, [answer]);
  useLayoutEffect(() => {
    const before = configureAccessCalls((event) => {
      if (event.kind !== "access_call") return false;
      const { request_id, call } = event.payload as { request_id: string; call: AccessCall };
      answerAccess(request_id, call);
      return true;
    });
    return () => void configureAccessCalls(before);
  }, [answerAccess]);

  useLayoutEffect(() => {
    const { rest, agents, focused, usage, frames, automations } = chapter.world.snapshot(model, Date.now());
    // The practice screen is the practice: its sidebar never invites to practice.
    const ui_state = { ...rest.ui_state, ...operator, practice_invite_closed: true };
    // A frame the world does not send is empty, so one chapter's files or records never show in the next.
    // A world with no missions holds an empty book, so the sidebar's 미션 row opens the quiet empty table, as in Hide.
    useShellStore.setState({
      rest: { missions: { items: [] }, ...rest, ui_state }, agents, connection: "live", focusedPaneId: focused, usage: usage ? { nodes: [usage] } : null,
      fileIndex: frames?.fileIndex ?? null, editor: frames?.editor ?? null, documents: frames?.documents ?? NO_DOCUMENTS,
      sessionWorkSearch: frames?.sessionWorkSearch ?? null, sessionRecord: frames?.sessionRecord ?? null, linkPanel: frames?.linkPanel ?? null,
      listings: frames?.listings ?? NO_LISTINGS, changes: frames?.changes ?? null, memory: frames?.memory ?? null, suggestions: frames?.suggestions ?? null,
      access: frames?.access ?? null,
      // The practice screen is this computer's own window, which hided sends its `pairing` frame (B52): nothing under way unless the world pairs.
      pairing: frames?.pairing ?? NOTHING_UNDER_WAY, pairingResult: frames?.pairingResult ?? null,
      automations: automations ?? null,
    });
  }, [chapter, model, operator]);

  useLayoutEffect(() => {
    useUiStore.setState({ screen: { kind: "workspace" }, overviewOpen: false, laneExpanded: false, stackFront: "view", automations: AUTOMATION_ENTRY });
  }, [chapter]);

  // A pane keeps no scrollback, so a terminal a layout change re-created or
  // shrank comes back blank; whichever terminal shows blank gets its screen
  // again, and a terminal whose screen the world changed is cleared and drawn anew.
  useEffect(() => {
    const wrote = new WeakMap<object, { at: number; text: string }>();
    const encode = (text: string) => btoa(String.fromCharCode(...new TextEncoder().encode(text)));
    const blank = (terminal: NonNullable<ReturnType<typeof terminalFor>>) => {
      const buffer = terminal.buffer.active;
      return buffer.cursorX === 0 && buffer.cursorY === 0 && !buffer.getLine(0)?.translateToString(true);
    };
    let frame = requestAnimationFrame(function write() {
      const now = performance.now();
      const { world, model: current } = live.current;
      const ready = world.panes(current).flatMap((id) => {
        const terminal = terminalFor(id);
        if (!terminal || terminal.rows <= 8) return [];
        const text = world.screen(current, id, terminal.cols);
        const last = wrote.get(terminal);
        const changed = last !== undefined && last.text !== text;
        const lost = blank(terminal) && now - (last?.at ?? 0) > 500;
        return changed || lost ? [{ id, terminal, text, changed }] : [];
      });
      if (ready.length) {
        feedChunks(ready.map(({ id, text, changed }) => ({ pane_id: id, bytes_base64: encode((changed ? CLEAR : "") + text) })));
        for (const { terminal, text } of ready) wrote.set(terminal, { at: now, text });
      }
      frame = requestAnimationFrame(write);
    });
    return () => cancelAnimationFrame(frame);
  }, []);

  return (
    <TooltipProvider>
      <div className="relative flex h-full min-w-0 flex-col bg-background text-foreground" data-practice-chapter={chapter.id} data-practice-moves={moves}>
        <TitleRow actions={actions} />
        <div className="flex min-h-0 min-w-0 flex-1">
          <Sidebar actions={actions} />
          {missionsShown ? <MissionScreen actions={actions} /> : automationsShown ? <AutomationScreen actions={actions} /> : <WorkspaceScreen actions={actions} />}
        </div>
        <OverviewModal actions={actions} />
        <Palette actions={actions} />
        {/* Settings opens over the practice as over Hide; what it sends goes to the world, which answers only what its chapter needs. */}
        <SettingsGate actions={actions} />
        <PairingDialogs actions={actions} />
        <CycleOverlay />
        <ConfirmClose actions={actions} />
        {/* A chapter about answering from the phone holds the phone beside the screen; its presses go to the world. */}
        {chapter.world.phone ? <PracticePhone screen={chapter.world.phone(model)} send={send} /> : null}
        <PracticeCoach chapter={chapter} model={model} next={next} resume={resume} onStep={onStep} onEnter={send} onNext={() => next && onStart(next, 0)} onRestart={() => onStart(chapter, 0)} onRewind={(step) => onStart(chapter, step)} onHub={onHub} onLeave={onLeave} />
      </div>
    </TooltipProvider>
  );
}
