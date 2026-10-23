// The practice window (onboarding practice mode): the production shell over
// the practice world, with the practice layer on top. The shell's own
// keyboard listener runs here, and every event it dispatches goes to
// `applyPractice` instead of a daemon, so a chord or a click changes the
// screen the way the core would and touches nothing outside the page.

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createActions } from "../actions";
import { GroupNameDialog } from "../AgentGroupControls";
import { TooltipProvider } from "../components/ui/tooltip";
import type { HostBridge } from "../host";
import { clientI18n } from "../i18n/translator";
import { installKeyboard } from "../keyboard";
import { CHAPTERS } from "../practice/chapters";
import { PracticeCoach } from "../practice/PracticeCoach";
import { Sidebar } from "../sidebar";
import { useShellStore } from "../store";
import { feedChunks, terminalFor } from "../terminals";
import { PALETTE_HIDE_CLASS } from "../theme";
import { TitleRow } from "../TitleRow";
import { useUiStore } from "../ui";
import { WorkspaceScreen } from "../WorkspaceScreen";

export type PracticeSceneParams = { chapter: number; theme: "light" | "dark"; system: "mac" | "pc" };

export function practiceSceneParams(params: URLSearchParams): PracticeSceneParams {
  const chapter = Number(params.get("chapter") ?? "0");
  if (!CHAPTERS.some((row) => row.number === chapter)) throw new Error(`Unknown practice chapter ${chapter}`);
  return { chapter, theme: params.get("theme") === "light" ? "light" : "dark", system: params.get("sys") === "pc" ? "pc" : "mac" };
}

/** The desktop host, standing in: its chords are the ones practice teaches, and it keeps no page and answers nothing. */
function standInHost(system: PracticeSceneParams["system"]): HostBridge {
  const quiet = (name: string) => (name.startsWith("on") ? () => () => {} : () => Promise.resolve(null));
  const part = <T extends object>(fixed: Record<string, unknown>): T => new Proxy(fixed, { get: (target, name) => (typeof name !== "string" || name === "then" ? undefined : name in target ? target[name] : quiet(name)) }) as T;
  return part<HostBridge>({ kind: "electron", platform: system === "mac" ? "darwin" : "win32", browser: part({}), updates: part({}) });
}

export function PracticeScene({ chapter: first, theme, system }: PracticeSceneParams) {
  if (!window.hideHost) window.hideHost = standInHost(system);
  const [number, setNumber] = useState(first);
  const chapter = CHAPTERS.find((row) => row.number === number)!;
  const next = CHAPTERS.find((row) => row.number === number + 1) ?? null;
  const [run, setRun] = useState(0);
  const [model, setModel] = useState<unknown>(() => chapter.world.initial());
  const live = useRef({ world: chapter.world, model });
  live.current = { world: chapter.world, model };

  const actions = useMemo(() => createActions((event) => {
    const after = live.current.world.apply(live.current.model, event);
    if (after) {
      live.current = { ...live.current, model: after };
      setModel(after);
    }
    return true;
  }), []);

  useEffect(() => installKeyboard(actions), [actions]);

  useLayoutEffect(() => {
    const { rest, agents, focused, usage } = chapter.world.snapshot(model, Date.now());
    useShellStore.setState({ rest, agents, connection: "live", focusedPaneId: focused, usage: usage ? { nodes: [usage] } : null });
  }, [chapter, model]);

  useLayoutEffect(() => {
    useUiStore.setState({ screen: { kind: "workspace" }, overviewOpen: false, laneShown: chapter.world.lane, laneExpanded: false, projectTab: "status", stackFront: "view" });
  }, [chapter, run]);

  useLayoutEffect(() => {
    const root = document.documentElement;
    root.classList.toggle("dark", theme === "dark");
    root.classList.toggle("light", theme === "light");
    root.classList.add(PALETTE_HIDE_CLASS);
    void clientI18n.changeLanguage("ko");
    root.lang = "ko";
    document.title = `hide · 연습 모드 ${chapter.number}편`;
  }, [chapter.number, theme]);

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
  }, [run]);

  const start = useCallback((target: typeof chapter) => {
    const fresh = target.world.initial();
    live.current = { world: target.world, model: fresh };
    setNumber(target.number);
    setModel(fresh);
    setRun((value) => value + 1);
  }, []);

  return (
    <TooltipProvider>
      <div className="flex h-full min-w-0 flex-col bg-background text-foreground" data-gallery-scene="practice" data-practice-chapter={chapter.number}>
        <TitleRow actions={actions} />
        <div className="flex min-h-0 min-w-0 flex-1">
          <Sidebar actions={actions} />
          <WorkspaceScreen key={run} actions={actions} />
        </div>
        <GroupNameDialog actions={actions} />
        <PracticeCoach key={run} chapter={chapter} model={model} next={next} onNext={() => next && start(next)} onRestart={() => start(chapter)} />
      </div>
    </TooltipProvider>
  );
}
