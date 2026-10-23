// One chapter playing: the production shell over the chapter's world, with
// the practice layer on top. The shell's own keyboard listener runs here, and
// every event it dispatches goes to the world instead of a daemon, so a chord
// or a click changes the screen the way the core would and touches nothing
// outside the page.

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { installKeyboard } from "../keyboard";
import { CycleOverlay } from "../Overlays";
import { OverviewModal } from "../Overview";
import { Palette } from "../Palette";
import type { SnapshotRest } from "../snapshot";
import { Sidebar } from "../sidebar";
import { useShellStore, type Documents } from "../store";
import { feedChunks, terminalFor } from "../terminals";
import { TitleRow } from "../TitleRow";
import { useUiStore } from "../ui";
import { WorkspaceScreen } from "../WorkspaceScreen";
import type { PracticeChapter } from "./chapter";
import { PracticeCoach } from "./PracticeCoach";
import { PRACTICE_COPY, type CoreEvent } from "./world";

type UiState = NonNullable<SnapshotRest["ui_state"]>;

const NO_DOCUMENTS: Documents = {};

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
  onStart: (chapter: PracticeChapter) => void;
  onHub: () => void;
};

export function PracticePlayer({ chapter, next, resume, operator, onStep, onStart, onHub }: Props) {
  const [model, setModel] = useState<unknown>(() => chapter.world.initial());
  // How many events the world has answered: the signal a step that takes several presses moves on.
  const [moves, setMoves] = useState(0);
  const live = useRef({ world: chapter.world, model });
  live.current = { world: chapter.world, model };

  const actions = useMemo(() => {
    const send = (event: CoreEvent) => {
      const after = live.current.world.apply(live.current.model, event);
      if (after) {
        live.current = { ...live.current, model: after };
        setModel(after);
        setMoves((count) => count + 1);
      }
      return true;
    };
    // A copy goes to the world, never to the operator's clipboard.
    return { ...createActions(send), copyText: (text: string, what: string) => void send({ kind: PRACTICE_COPY, payload: { text, what } }) };
  }, []);

  useEffect(() => installKeyboard(actions), [actions]);

  useLayoutEffect(() => {
    const { rest, agents, focused, usage, frames } = chapter.world.snapshot(model, Date.now());
    const ui_state = { ...rest.ui_state, ...operator };
    // A frame the world does not send is empty, so one chapter's files never show in the next.
    useShellStore.setState({
      rest: { ...rest, ui_state }, agents, connection: "live", focusedPaneId: focused, usage: usage ? { nodes: [usage] } : null,
      fileIndex: frames?.fileIndex ?? null, editor: frames?.editor ?? null, documents: frames?.documents ?? NO_DOCUMENTS,
    });
  }, [chapter, model, operator]);

  useLayoutEffect(() => {
    useUiStore.setState({ screen: { kind: "workspace" }, overviewOpen: false, laneShown: chapter.world.lane, laneExpanded: false, stackFront: "view" });
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
      <div className="flex h-full min-w-0 flex-col bg-background text-foreground" data-practice-chapter={chapter.id} data-practice-moves={moves}>
        <TitleRow actions={actions} />
        <div className="flex min-h-0 min-w-0 flex-1">
          <Sidebar actions={actions} />
          <WorkspaceScreen actions={actions} />
        </div>
        <OverviewModal actions={actions} />
        <Palette actions={actions} />
        <CycleOverlay />
        <PracticeCoach chapter={chapter} model={model} next={next} resume={resume} onStep={onStep} onNext={() => next && onStart(next)} onRestart={() => onStart(chapter)} onHub={onHub} />
      </div>
    </TooltipProvider>
  );
}
