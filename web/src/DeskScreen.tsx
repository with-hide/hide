// Desk's room on screen (PRD session-docs-and-assistant, Desk 1차 B1-B24):
// the `desk`, `desk_live`, `desk_jobs` and `desk_mentions` sections drawn
// with the Desk parts, and each press sent as one `desk_*` event. The core
// decides every state; this screen keeps only what the operator is writing
// and where the `@` menu is. The lane (B28-B35) is `DeskLaneColumns` below:
// its pin and kept tabs are the core's, its columns this window's.

import { MapIcon } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode, type RefObject } from "react";
import type { Actions } from "./actions";
import { askingCount, deskOn, laneCards, sessionFacts, sessionThreadId, threadKeyPane, unseenOutputs, type DeskItem, type DeskJob, type DeskMention, type DeskOutput, type DeskSessionRef } from "./desk";
import { activeTab, backFromThread, namesView, closeLane, closeTab, closeTool, closeView, openMission, openThread, outputTab, showTab, showThreads, tabKey, toggleOutputs, toggleThreads, viewTabs } from "./deskLane";
import { DeskOutputView } from "./DeskOutputView";
import { MissionMap } from "./MissionMap";
import { OutputMark } from "./components/desk/desk-job-card";
import { EveryOutput } from "./components/desk/desk-outputs";
import { ThreadList } from "./components/desk/thread-list";
import { ThreadPanel } from "./components/desk/thread-panel";
import { SessionChipProvider, type SessionChipSource } from "./components/desk/session-chip";
import { allAgents } from "./paneAgents";
import type { DeskKeptTab } from "./snapshot";
import { commandLabel } from "./shortcutLabels";
import { leaveDesk } from "./keyboard";
import { mentionAt, mentionRows, outgoing, pickMention } from "./deskComposer";
import { DeskChat, DeskEmpty, type DeskChatActions, type DeskNoticeAction } from "./components/desk/desk-chat";
import { DeskAgentBar, DeskComposer } from "./components/desk/desk-composer";
import { DeskPaneReturn, DeskSidebarRow } from "./components/desk/desk-entry";
import { DeskChatColumn, DeskChatHead, DeskRail, DeskRoom, DeskViewColumn, LaneColumnHead, LaneHeadControls, type ViewTab } from "./components/desk/desk-lane";
import { DeskMentionMenu, type MentionProject, type MentionSession } from "./components/desk/desk-mention-menu";
import { DeskSettingsTab } from "./components/desk/desk-settings";
import { useInterfaceTranslation } from "./i18n/client";
import { catalogWorkspaces } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { windowIsActive } from "./viewFocus";

const NO_JOBS: DeskJob[] = [];
const NO_TABS: DeskKeptTab[] = [];

/** The sidebar's `◆ Desk` row (B1), once hided runs Desk and Settings › Desk switched it on (B41). */
export function DeskSidebarEntry({ actions }: { actions: Actions }) {
  const shown = useShellStore((s) => deskOn(s.desk));
  const selected = useUiStore((s) => s.screen?.kind === "desk");
  const asking = useShellStore((s) => askingCount(s.deskJobs?.jobs ?? NO_JOBS));
  if (!shown) return null;
  return (
    <div className="shrink-0 px-xs pt-xs">
      <DeskSidebarRow selected={selected} asking={asking} onOpen={actions.openDesk} />
    </div>
  );
}

/** Settings › Desk (B4, B19, B21). */
export function DeskSettingsPage({ actions }: { actions: Actions }) {
  const desk = useShellStore((s) => s.desk);
  if (!desk) return null;
  return <DeskSettingsTab settings={desk.settings} agents={desk.agents} onChange={actions.deskSettings} />;
}

/** A conversation this close to its end follows what arrives. */
const STICK_PX = 48;

/**
 * Reads the job cards the operator can see (a card read is its row read, as
 * the sidebar reads it): cards in the conversation's view while this window
 * is in front, each change of a card once.
 */
function useReadShownCards(scroller: RefObject<HTMLDivElement | null>, jobs: readonly DeskJob[], onRead: (id: string) => void) {
  const sent = useRef(new Set<string>());
  useEffect(() => {
    const box = scroller.current;
    if (!box) return;
    let frame = 0;
    const read = () => {
      frame = 0;
      if (document.visibilityState !== "visible" || !windowIsActive()) return;
      const view = box.getBoundingClientRect();
      const unread = new Map(jobs.filter((job) => !job.read).map((job) => [job.id, job]));
      for (const card of Array.from(box.querySelectorAll<HTMLElement>("[data-desk-job-id]"))) {
        const job = unread.get(card.dataset.deskJobId!);
        if (!job) continue;
        const rect = card.getBoundingClientRect();
        if (rect.bottom <= view.top || rect.top >= view.bottom) continue;
        const key = `${job.id}\u0000${job.changed_at_unix_ms}`;
        if (sent.current.has(key)) continue;
        sent.current.add(key);
        onRead(job.id);
      }
    };
    const soon = () => {
      if (!frame) frame = requestAnimationFrame(read);
    };
    soon();
    box.addEventListener("scroll", soon, { passive: true });
    window.addEventListener("focus", soon);
    document.addEventListener("visibilitychange", soon);
    return () => {
      if (frame) cancelAnimationFrame(frame);
      box.removeEventListener("scroll", soon);
      window.removeEventListener("focus", soon);
      document.removeEventListener("visibilitychange", soon);
    };
  }, [scroller, jobs, onRead]);
}

/** Where the `@` menu stands: the `@` word Esc closed it on, and the project ⇥ entered. */
type MenuPlace = { dismissed: number | null; entered: string | null; active: string };

export function DeskScreen({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const desk = useShellStore((s) => s.desk);
  // Switched off while in front, the room goes and the panel before it comes back (B41).
  const off = desk !== null && !deskOn(desk);
  useEffect(() => {
    if (off) leaveDesk(actions);
  }, [off, actions]);
  const live = useShellStore((s) => s.deskLive);
  const jobs = useShellStore((s) => s.deskJobs?.jobs ?? NO_JOBS);
  const mentions = useShellStore((s) => s.deskMentions);
  const draft = useUiStore((s) => s.deskDraft);
  const setDraft = useUiStore((s) => s.setDeskDraft);
  const root = useRef<HTMLDivElement>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const [caret, setCaret] = useState(draft.text.length);
  const [menu, setMenu] = useState<MenuPlace>({ dismissed: null, entered: null, active: "" });
  // The words a send carried, held until the conversation shows them, so a refused send keeps them in the box.
  const pending = useRef<{ text: string; after: number } | null>(null);
  // The last send as written, put back in the box when the agent turned out not to be signed in (B6).
  const lastSent = useRef<{ words: typeof draft; after: number } | null>(null);

  const byId = useMemo(() => Object.fromEntries(jobs.map((job) => [job.id, job])), [jobs]);
  const items = desk?.conversation.items;
  // A card past the newest 64 the section carries is not drawn; its line in the conversation goes with it.
  const shown = useMemo(() => (items ?? []).filter((item) => item.kind !== "job" || byId[item.job_id] !== undefined), [items, byId]);
  const turn = live?.started_at_ms ?? null;
  const answering = turn !== null;
  const liveText = live?.text ? live.text : null;
  const agent = desk?.selection.agent ? desk.agents.find((one) => one.id === desk.selection.agent) : undefined;
  const canSend = !answering && (agent?.state === "ready" || agent?.state === "login_needed");

  const lastUserId = (list: readonly DeskItem[]) => list.reduce((last, item) => (item.kind === "user" ? item.id : last), -1);
  useEffect(() => {
    const sent = pending.current;
    if (!sent || !items) return;
    if (items.some((item) => item.kind === "user" && item.id > sent.after && item.text === sent.text)) {
      pending.current = null;
      if (outgoing(useUiStore.getState().deskDraft).text === sent.text) setDraft({ text: "", mentions: [] });
    }
  }, [items, setDraft]);
  useEffect(() => {
    const sent = lastSent.current;
    if (!sent || !items?.some((item) => item.kind === "notice" && item.notice === "login_needed" && item.id > sent.after)) return;
    lastSent.current = null;
    if (!useUiStore.getState().deskDraft.text) setDraft(sent.words);
  }, [items, setDraft]);

  useLayoutEffect(() => {
    const box = scroller.current;
    if (box && stick.current) box.scrollTop = box.scrollHeight;
  }, [shown, liveText, answering]);
  useEffect(() => {
    const box = scroller.current;
    if (!box) return;
    const follow = () => {
      stick.current = box.scrollHeight - box.scrollTop - box.clientHeight < STICK_PX;
    };
    box.addEventListener("scroll", follow, { passive: true });
    return () => box.removeEventListener("scroll", follow);
  }, []);

  useReadShownCards(scroller, jobs, actions.deskJobRead);

  const send = (words = draft) => {
    if (!canSend || !items) return;
    const out = outgoing(words);
    if (!out.text) return;
    pending.current = { text: out.text, after: lastUserId(items) };
    lastSent.current = { words, after: items.reduce((last, item) => Math.max(last, item.id), -1) };
    stick.current = true;
    actions.deskSend(out.text, out.mentions);
  };

  // The `@` menu: open while the caret ends an `@` word Esc did not close, and something matches it.
  const at = mentionAt(draft.text, caret);
  const entered = menu.entered !== null ? mentions?.projects.find((project) => project.id === menu.entered) ?? null : null;
  const rows = at && mentions ? mentionRows(mentions, at.query) : null;
  const enteredSessions = entered && mentions ? mentions.sessions.filter((session) => entered.sessions.includes(session.id)) : [];
  const values = entered ? [`new:${entered.id}`, ...enteredSessions.map((session) => `session:${session.id}`)] : rows ? [...rows.sessions.map((session) => `session:${session.id}`), ...rows.projects.map((project) => `project:${project.id}`)] : [];
  const menuOpen = at !== null && menu.dismissed !== at.start && values.length > 0;
  const active = values.includes(menu.active) ? menu.active : (values[0] ?? "");

  const input = () => root.current?.querySelector<HTMLTextAreaElement>("[data-desk-composer-input]") ?? null;
  const place = (next: number) => {
    setCaret(next);
    requestAnimationFrame(() => input()?.setSelectionRange(next, next));
  };
  const pick = (mention: DeskMention) => {
    if (!at) return;
    const next = pickMention(draft, at, mention);
    setDraft(next.draft);
    setMenu({ dismissed: null, entered: null, active: "" });
    place(next.caret);
    input()?.focus();
  };
  const pickSession = (session: MentionSession) => pick({ kind: "session", id: session.id, name: session.name, mark: session.mark, agent_kind: session.agent_kind });
  const pickProject = (project: MentionProject) => pick({ kind: "project", id: project.id, name: project.name });
  const choose = (value: string) => {
    const [kind, ...rest] = value.split(":");
    const id = rest.join(":");
    if (kind === "session") {
      const session = mentions?.sessions.find((one) => one.id === id);
      if (session) pickSession(session);
    } else {
      const project = mentions?.projects.find((one) => one.id === id);
      if (project) pickProject(project);
    }
  };

  // The `@` menu's keys; the composer sends on ↵ when the menu is closed.
  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.nativeEvent.isComposing || !menuOpen) return;
    const key = event.key;
    if (key === "ArrowDown" || key === "ArrowUp") {
      event.preventDefault();
      const index = values.indexOf(active);
      const next = (index + (key === "ArrowDown" ? 1 : values.length - 1)) % values.length;
      setMenu({ ...menu, active: values[next]! });
    } else if (key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      choose(active);
    } else if (key === "Tab" && !event.shiftKey && active.startsWith("project:")) {
      event.preventDefault();
      const id = active.slice("project:".length);
      setMenu({ ...menu, entered: id, active: `new:${id}` });
    } else if ((key === "Tab" && event.shiftKey) || key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      setMenu(menu.entered !== null ? { ...menu, entered: null, active: "" } : { ...menu, dismissed: at!.start });
    }
  };
  const track = () => {
    const box = input();
    if (box) setCaret(box.selectionStart);
  };

  const onNoticeAction = useCallback((action: DeskNoticeAction, notice: Extract<DeskItem, { kind: "notice" }>) => {
    const now = useShellStore.getState().desk;
    switch (action) {
      case "restart":
        return actions.deskRestart();
      case "reinstall": {
        const which = notice.agent ?? now?.selection.agent ?? null;
        if (which) actions.deskInstall(which);
        return;
      }
      case "retry": {
        // 다시 시도 sends the operator's last message again, as they wrote it.
        const last = [...(now?.conversation.items ?? [])].reverse().find((item) => item.kind === "user");
        if (last?.kind === "user") actions.deskSend(last.text, last.mentions);
        return;
      }
    }
  }, [actions]);
  const lane = useLaneActions(actions);
  const chipSource = useSessionChipSource(lane);
  const chatActions = useMemo<DeskChatActions>(() => ({
    onOpen: (job) => lane.thread(job),
    onAnswer: (job) => lane.thread(job, true),
    onCancel: (job) => actions.deskJobCancel(job.id),
    onRequeue: (job) => actions.deskJobRequeue(job.id),
    onRetry: (job) => actions.deskJobRequeue(job.id),
    onOutput: lane.output,
    onMission: lane.mission,
    onApproval: (id, allow) => actions.deskApproval(id, allow),
    onNoticeAction,
    onCopy: (text) => void actions.copyText(text, "desk sign-in command"),
    onMention: (mention) => {
      if (mention.kind === "session") return lane.session({ name: mention.name, mark: mention.mark ?? null, agent_kind: mention.agent_kind ?? null, pane_id: mention.id });
      const rest = useShellStore.getState().rest;
      const project = catalogWorkspaces(rest).find((row) => row.checkouts.some((checkout) => checkout.path === mention.id));
      const checkout = project?.checkouts.find((row) => row.path === mention.id);
      if (project && checkout) actions.openWorkspace(project.device_id, project.id, checkout.id);
    },
    onHanded: (paneId) => lane.session({ name: "", mark: null, agent_kind: null, pane_id: paneId }),
  }), [actions, onNoticeAction, lane]);
  const laneWindow = useUiStore((s) => s.deskLane);

  if (!desk) return null;
  const working = jobs.filter((job) => job.state === "working").length;
  const shownThread = laneWindow.up && laneWindow.tool?.kind === "thread" ? laneWindow.tool.job : null;
  const shownTab = laneWindow.up && laneWindow.view ? activeTab(desk.lane.kept, laneWindow) : null;
  const shownOutput = shownTab?.kind === "output" ? shownTab.output.id : null;
  const composer = (
    <DeskComposer
      value={draft.text}
      label={t("desk.composer.label")}
      placeholder={t("desk.composer.placeholder")}
      answering={answering}
      held={!canSend}
      focused={menuOpen}
      menuOpen={menuOpen}
      menu={menuOpen ? (
        <DeskMentionMenu
          sessions={rows?.sessions ?? []}
          projects={rows?.projects ?? []}
          entered={entered ? { project: entered, sessions: enteredSessions } : null}
          active={active}
          onActive={(value) => setMenu({ ...menu, active: value })}
          onSession={pickSession}
          onProject={pickProject}
          onBack={() => setMenu({ ...menu, entered: null, active: "" })}
        />
      ) : null}
      onMenuOpenChange={(open) => {
        if (!open && at) setMenu({ ...menu, dismissed: at.start });
      }}
      onChange={(text) => {
        setDraft({ ...draft, text });
        requestAnimationFrame(track);
      }}
      onKeyDown={onKeyDown}
      onSend={() => send()}
      onStop={actions.deskStop}
    >
      <DeskAgentBar agents={desk.agents} selection={desk.selection} onAgent={(id) => actions.deskSelect(id, null, null)} onModel={(model) => actions.deskSelect(null, model, null)} onEffort={(effort) => actions.deskSelect(null, null, effort)} />
    </DeskComposer>
  );
  return (
    <SessionChipProvider source={chipSource}>
    <div ref={root} className="flex min-h-0 min-w-0 flex-1" data-desk-screen="true" onKeyUp={track} onMouseUp={track}>
      <DeskLaneColumns
        actions={actions}
        lane={lane}
        chat={(
          <DeskChatColumn head={<DeskChatHead onNewConversation={actions.deskNewConversation} />} composer={composer} scrollRef={scroller}>
            {shown.length === 0 && !answering ? (
              <DeskEmpty working={working} asking={askingCount(jobs)} onExample={(text) => (canSend ? send({ text, mentions: [] }) : setDraft({ text, mentions: [] }))} onShowThreads={lane.threads} />
            ) : (
              <DeskChat items={shown} jobs={byId} agents={desk.agents} live={liveText} thinking={answering && liveText === null} startedAt={turn} selectedJob={shownThread} selectedOutput={shownOutput} {...chatActions} />
            )}
          </DeskChatColumn>
        )}
      />
    </div>
    </SessionChipProvider>
  );
}

/**
 * Where Desk's chips read their sessions (D-86): the sidebar's rows and
 * every card the lane can open, so a chip in the chat or a thread shows its
 * row's mark, dims once the row is gone, and opens its card under the
 * pointer; a press opens its thread.
 */
function useSessionChipSource(lane: LaneActions): SessionChipSource {
  const rows = useShellStore((s) => allAgents(s.agents, s.rest?.status?.remote));
  const section = useShellStore((s) => s.deskJobs);
  return useMemo(() => {
    const byPane = new Map(rows.map((row) => [row.pane_id, row]));
    const cards = laneCards(section);
    return { facts: (session) => sessionFacts(byPane, cards, session), onOpen: lane.session };
  }, [rows, section, lane]);
}

const NO_WORKERS: DeskJob[] = [];

/** Every thread Desk shows: its jobs, then the workers of the missions they conduct (B28, B34). */
function useThreads(): readonly DeskJob[] {
  const jobs = useShellStore((s) => s.deskJobs?.jobs ?? NO_JOBS);
  const workers = useShellStore((s) => s.deskJobs?.workers ?? NO_WORKERS);
  return useMemo(() => (workers.length ? [...jobs, ...workers] : jobs), [jobs, workers]);
}

/** Every card the lane opens as a thread: the Threads list's, and the other sessions the conversation names (D-86). */
function everyThread(): DeskJob[] {
  return laneCards(useShellStore.getState().deskJobs);
}

/** The output a thread's own line names, as its card lists it when it does: the View's id, or its address or file. */
function outputOf(job: DeskJob, output: DeskOutput): DeskOutput {
  return job.outputs.find((made) => made.id === output.id || (output.url !== null && made.url === output.url) || (output.path !== null && namesView(made, output.path))) ?? output;
}

/** What a press in Desk opens in its lane (B25, B28, B29, B31, B34). */
type LaneActions = {
  thread: (job: DeskJob, answer?: boolean) => void;
  threads: () => void;
  output: (output: DeskOutput) => void;
  threadOutput: (job: DeskJob, output: DeskOutput) => void;
  mission: (missionId: string) => void;
  session: (session: DeskSessionRef) => void;
  node: (paneId: string | null, conductor: string | null) => void;
};

function useLaneActions(actions: Actions): LaneActions {
  return useMemo(() => {
    const set = (change: Parameters<ReturnType<typeof useUiStore.getState>["setDeskLane"]>[0]) => useUiStore.getState().setDeskLane(change);
    const kept = () => useShellStore.getState().desk?.lane.kept ?? NO_TABS;
    const thread = (job: DeskJob, answer = false) => set((lane) => openThread(lane, job.id, { answer }));
    const threadOutput = (job: DeskJob, output: DeskOutput) => set((lane) => showTab(lane, kept(), outputTab(job.id, outputOf(job, output))));
    return {
      thread,
      threads: () => set(showThreads),
      output: (output) => {
        const job = everyThread().find((one) => one.outputs.some((made) => made.id === output.id));
        if (job) threadOutput(job, output);
      },
      threadOutput,
      mission: (missionId) => {
        const conductor = everyThread().find((job) => job.mission?.id === missionId) ?? null;
        const title = conductor?.mission?.title ?? useShellStore.getState().rest?.missions?.items.find((one) => one.id === missionId)?.title ?? missionId;
        set((lane) => openMission(lane, kept(), { id: missionId, title }, conductor?.id ?? null));
      },
      // Any session opens as its thread in the lane and Desk stays in front (D-87, B51): a job or a
      // worker by its card, any other by its pane; its tab is the thread's `터미널로` and ⌘J.
      session: (session) => {
        const job = everyThread().find((one) => (session.pane_id ? one.pane_id === session.pane_id : one.title === session.name));
        if (job) return thread(job);
        const state = useShellStore.getState();
        const pane = session.pane_id
          ?? allAgents(state.agents, state.rest?.status?.remote).find((row) => row.identity_label === session.name)?.pane_id
          ?? state.deskMentions?.sessions.find((one) => one.name === session.name)?.id;
        if (!pane) return state.noteDiagnostic("desk session: no pane for the session pressed");
        set((lane) => openThread(lane, sessionThreadId(pane)));
      },
      // A mission map's node (B34): the tool column turns to its worker's thread, the path line leading back to the Conductor.
      node: (paneId, conductor) => {
        const job = paneId ? everyThread().find((one) => one.pane_id === paneId) : undefined;
        if (job) set((lane) => openThread(lane, job.id, { conductor }));
      },
    };
  }, [actions]);
}

/**
 * Desk's room with its lane (D-64, D-73, D-74, B28-B35): the chat, the 보기
 * column with the outputs and mission maps as tabs, the tool column with
 * the Threads list or one thread, and the rail. Escape on an unpinned lane
 * takes it down as ⌘E does and gives the keyboard back to the composer.
 */
function DeskLaneColumns({ actions, lane, chat }: { actions: Actions; lane: LaneActions; chat: ReactNode }) {
  const { t } = useInterfaceTranslation();
  const room = useShellStore((s) => s.desk?.lane);
  const own = useShellStore((s) => s.deskJobs?.jobs ?? NO_JOBS);
  const threads = useThreads();
  const place = useUiStore((s) => s.deskLane);
  const set = useUiStore((s) => s.setDeskLane);
  const chord = useShellStore((s) => commandLabel("desk_terminal", s.rest?.ui_state)) || null;
  const [conductors, setConductors] = useState<ReadonlySet<string>>(() => new Set());
  const kept = room?.kept ?? NO_TABS;
  const pinned = room?.pinned ?? false;
  const pin = () => actions.deskLanePin(!pinned);
  const expand = () => set((now) => ({ ...now, expanded: !now.expanded }));
  const tabs = viewTabs(kept, place);
  const active = activeTab(kept, place);
  const named = useShellStore((s) => s.deskJobs?.named ?? NO_WORKERS);
  const byId = new Map([...threads, ...named].map((job) => [job.id, job]));
  // Opening a thread reads it, as reading its card does; a worker is not Desk's job and has no card to read.
  const openJob = place.up && place.tool?.kind === "thread" ? place.tool.job : null;
  const unread = openJob !== null && own.some((job) => job.id === openJob && !job.read) ? openJob : null;
  useEffect(() => {
    if (unread) actions.deskJobRead(unread);
  }, [actions, unread]);

  const list = (
    <>
      <LaneColumnHead pinned={pinned} expanded={place.expanded} onExpand={expand} onPin={pin} onClose={() => set(closeTool)}>
        <span className="font-semibold text-foreground">{t("desk.threads.title")}</span>
      </LaneColumnHead>
      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto">
        <ThreadList
          jobs={threads}
          expanded={conductors}
          onOpen={(job) => lane.thread(job)}
          onMission={lane.mission}
          onOutput={lane.output}
          onToggle={(id) => setConductors((open) => {
            const next = new Set(open);
            if (next.has(id)) next.delete(id);
            else next.add(id);
            return next;
          })}
        />
      </div>
    </>
  );
  let tool: ReactNode = null;
  if (place.up && place.tool) {
    const key = place.tool.kind === "thread" ? place.tool.job : null;
    const shown = key !== null ? byId.get(key) : undefined;
    // A session that is no job of Desk's opens by its pane before its card arrives (D-87).
    const paneId = shown?.pane_id ?? (key !== null ? threadKeyPane(key) : null);
    if (key !== null && paneId && place.tool.kind === "thread") {
      const conductor = place.tool.conductor ? byId.get(place.tool.conductor) : undefined;
      tool = (
        <ThreadPanel
          key={key}
          job={shown ?? null}
          paneId={paneId}
          actions={actions}
          conductor={conductor ? { title: conductor.title, onOpen: () => set(backFromThread) } : null}
          pinned={pinned}
          startedByDesk={own.some((job) => job.id === key)}
          selectedOutput={active?.kind === "output" && active.job === key ? active.output.id : null}
          answer={place.answer?.job === key ? place.answer.seq : 0}
          terminalChord={chord}
          onBack={() => set(backFromThread)}
          onPin={pin}
          onClose={() => set(closeTool)}
          onTerminal={() => actions.openAgent(paneId)}
          onMission={lane.mission}
          onOutput={(output) => (shown ? lane.threadOutput(shown, output) : undefined)}
          onSession={lane.session}
        />
      );
    } else {
      // A thread that left Desk (moved out, past the cards' cap) gives its place back to the list.
      tool = list;
    }
  }

  let view: ReactNode = null;
  if (place.up && place.view) {
    const viewTabsShown: ViewTab[] = tabs.map(({ tab, preview }) => ({
      id: tabKey(tab),
      label: tab.kind === "output" ? tab.output.label : t("desk.lane.missionMap", { title: tab.title }),
      preview,
      icon: tab.kind === "output" ? <OutputMark output={tab.output} /> : <MapIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />,
    }));
    const tabOf = (key: string) => tabs.find(({ tab }) => tabKey(tab) === key);
    let body: ReactNode;
    if (active?.kind === "output") {
      // The card's value now, so a View seen since shows no dot; a kept tab whose View
      // closed has no View left to mark, so it reads as seen and asks the core nothing.
      const output = byId.get(active.job)?.outputs.find((one) => one.id === active.output.id) ?? { ...active.output, seen: true };
      body = <DeskOutputView key={tabKey(active)} output={output} viewKey={tabKey(active)} actions={actions} />;
    } else if (active?.kind === "mission") {
      const conductor = threads.find((job) => job.mission?.id === active.mission)?.id ?? null;
      body = <MissionMap missionId={active.mission} actions={actions} onNode={(node) => lane.node(node.pane_id ?? null, conductor)} />;
    } else {
      body = <EveryOutput threads={threads} onOpen={lane.threadOutput} />;
    }
    view = (
      <DeskViewColumn
        tabs={viewTabsShown}
        active={active ? tabKey(active) : null}
        onTab={(key) => set((now) => ({ ...now, view: true, active: key }))}
        onKeep={(key) => {
          const found = tabOf(key);
          if (found?.preview) actions.deskTabKeep(found.tab, true);
        }}
        onCloseTab={(key) => {
          const found = tabOf(key);
          if (!found) return;
          set((now) => closeTab(now, kept, found.tab));
          if (!found.preview) actions.deskTabKeep(found.tab, false);
        }}
        controls={<LaneHeadControls edge={tool === null} expanded={place.expanded} pinned={pinned} onExpand={expand} onPin={pin} onClose={() => set(closeView)} />}
      >
        {body}
      </DeskViewColumn>
    );
  }

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "Escape" || event.defaultPrevented) return;
    if (place.expanded) set((now) => ({ ...now, expanded: false }));
    else if (place.up && !pinned) {
      set(closeLane);
      document.querySelector<HTMLTextAreaElement>("[data-desk-screen] [data-desk-chat-column] [data-desk-composer-input]")?.focus();
    } else return;
    event.preventDefault();
    event.stopPropagation();
  };
  return (
    <div className="contents" onKeyDown={onKeyDown}>
      <DeskRoom
        chat={chat}
        view={view}
        tool={tool}
        pinned={pinned}
        expanded={place.expanded}
        rail={<DeskRail asking={askingCount(threads)} unseen={unseenOutputs(threads)} threads={place.up && place.tool !== null} outputs={place.up && place.view} onThreads={() => set(toggleThreads)} onOutputs={() => set(toggleOutputs)} />}
      />
    </div>
  );
}

/**
 * At the head of a Desk thread's Agent tab (D-42, B35, B36): `나도 참여 중`
 * once the operator typed into it, and `Desk로`, which ⌘J also presses, back
 * to Desk as it was left with this thread in its lane.
 */
export function PaneDeskReturn({ paneId, actions }: { paneId: string; actions: Actions }) {
  const job = useShellStore((s) => s.deskJobs?.jobs.find((one) => one.pane_id === paneId) ?? s.deskJobs?.workers.find((one) => one.pane_id === paneId) ?? null);
  const chord = useShellStore((s) => commandLabel("desk_terminal", s.rest?.ui_state)) || null;
  const on = useShellStore((s) => deskOn(s.desk));
  if (!job || !on) return null;
  return <DeskPaneReturn joined={job.joined} chord={chord} onDesk={() => actions.openDeskThread(job.id)} />;
}
