// One chapter playing: the production shell over the chapter's world, with
// the practice layer on top. The shell's own keyboard listener runs here, and
// every event it dispatches goes to the world instead of a daemon, so a chord
// or a click changes the screen the way the core would and touches nothing
// outside the page.

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { installKeyboard } from "../keyboard";
import { Palette } from "../Palette";
import type { SnapshotRest } from "../snapshot";
import { Sidebar } from "../sidebar";
import { useShellStore } from "../store";
import { feedChunks, terminalFor } from "../terminals";
import { TitleRow } from "../TitleRow";
import { useUiStore } from "../ui";
import { WorkspaceScreen } from "../WorkspaceScreen";
import type { PracticeChapter } from "./chapter";
import { PracticeCoach } from "./PracticeCoach";

type UiState = NonNullable<SnapshotRest["ui_state"]>;

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

  const actions = useMemo(() => createActions((event) => {
    const after = live.current.world.apply(live.current.model, event);
    if (after) {
      live.current = { ...live.current, model: after };
      setModel(after);
      setMoves((count) => count + 1);
    }
    return true;
  }), []);

  useEffect(() => installKeyboard(actions), [actions]);

  useLayoutEffect(() => {
    const { rest, agents, focused, usage, frames } = chapter.world.snapshot(model, Date.now());
    const ui_state = { ...rest.ui_state, ...operator };
    // A frame the world does not send is empty, so one chapter's records never show in the next.
    useShellStore.setState({
      rest: { ...rest, ui_state }, agents, connection: "live", focusedPaneId: focused, usage: usage ? { nodes: [usage] } : null,
      sessionWorkSearch: frames?.sessionWorkSearch ?? null, sessionRecord: frames?.sessionRecord ?? null, linkPanel: frames?.linkPanel ?? null,
    });
  }, [chapter, model, operator]);

  useLayoutEffect(() => {
    useUiStore.setState({ screen: { kind: "workspace" }, overviewOpen: false, laneShown: chapter.world.lane, laneExpanded: false, stackFront: "view" });
  }, [chapter]);

  // A pane keeps no scrollback, so a terminal a layout change re-created or
  // shrank comes back blank; whichever terminal shows blank gets its screen again.
  useEffect(() => {
    const wrote = new WeakMap<object, number>();
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
        return terminal && terminal.rows > 8 && blank(terminal) && now - (wrote.get(terminal) ?? 0) > 500 ? [{ id, terminal }] : [];
      });
      if (ready.length) {
        feedChunks(ready.map(({ id, terminal }) => ({ pane_id: id, bytes_base64: encode(world.screen(current, id, terminal.cols)) })));
        for (const { terminal } of ready) wrote.set(terminal, now);
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
        <Palette actions={actions} />
        <PracticeCoach chapter={chapter} model={model} next={next} resume={resume} onStep={onStep} onNext={() => next && onStart(next)} onRestart={() => onStart(chapter)} onHub={onHub} />
      </div>
    </TooltipProvider>
  );
}
