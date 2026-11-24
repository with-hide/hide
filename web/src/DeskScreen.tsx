// Desk's room on screen (PRD session-docs-and-assistant, Desk 1차; PRD
// desk-2-ui): the `desk`, `desk_live`, `desk_jobs` and `desk_mentions`
// sections drawn with the Desk parts, each press sent as one `desk_*` event,
// and at the chat's right the session opened beside Desk with the tab panel
// floating at its left edge (`DeskRoom`). The core decides every state; this
// screen keeps what the operator is writing, where the `@` menu is, and the
// room's window state (`deskRoom.ts`), which the core keeps only the widths of.

import { MapIcon } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type RefObject } from "react";
import type { Actions } from "./actions";
import { deskOn, laneCards, sessionFacts, type DeskItem, type DeskJob, type DeskMention, type DeskOutput, type DeskSessionRef } from "./desk";
import { activeTab, closePanel, closeSession, closeTab, keepTab, namesView, openSession, showTab, tabKey, tabNames, type DeskTab } from "./deskRoom";
import { DeskOutputView } from "./DeskOutputView";
import { DisplayEditor } from "./Editor";
import { ExplorerTree } from "./ExplorerTree";
import { MissionMap } from "./MissionMap";
import { OutputMark } from "./components/desk/desk-job-card";
import { ThreadPanel } from "./components/desk/thread-panel";
import { SessionChipProvider, type SessionChipSource } from "./components/desk/session-chip";
import { SessionAskCard, SessionHead, SessionQuestionCard, SessionSleepingFoot, sessionQuestion } from "./components/desk/desk-session";
import { useElapsed } from "./components/elapsed";
import { askIdentity, inputFailure, noticeText } from "./mobile/protocol";
import { allAgents, agentsByPane } from "./paneAgents";
import { commandLabel } from "./shortcutLabels";
import { leaveDesk } from "./keyboard";
import { mentionAt, mentionTree, outgoing, pickMention } from "./deskComposer";
import { DeskChat, DeskEmpty, type DeskChatActions, type DeskNoticeAction } from "./components/desk/desk-chat";
import { DeskAgentBar, DeskComposer } from "./components/desk/desk-composer";
import type { MentionInputHandle } from "./components/desk/mention-input";
import { DeskPaneReturn } from "./components/desk/desk-entry";
import { DeskChatColumn, DeskChatHead, DeskRoom, DeskTabPanel, type PanelTab } from "./components/desk/desk-room";
import { DeskMentionMenu, type MentionProject, type MentionSession } from "./components/desk/desk-mention-menu";
import { DeskSettingsTab } from "./components/desk/desk-settings";
import { FileMark } from "./components/file-mark";
import { fileIcon } from "./fileIcons";
import { useEscapeLayer } from "./components/ui/layer";
import { useInterfaceTranslation } from "./i18n/client";
import { machineName } from "./screenMachine";
import { catalogWorkspaces, explorerContext, frontCheckout, localDeviceId, type AgentRow, type PaneRow, type SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { areasOf, placeKey, workspaceKey } from "./viewLayout";
import { windowIsActive } from "./viewFocus";
import { workspaceViewOf } from "./workspace";

const NO_JOBS: DeskJob[] = [];

/** Settings › Labs › Desk (B4, B19, B21). */
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

/** Where the `@` menu stands: the `@` word Esc closed it on, the project ⇥ entered, and the parents the operator folded open or shut. */
type MenuPlace = { dismissed: number | null; entered: string | null; active: string; folds: Record<string, boolean> };

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
  const scroller = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const [caret, setCaret] = useState(draft.text.length);
  const [menu, setMenu] = useState<MenuPlace>({ dismissed: null, entered: null, active: "", folds: {} });
  const box = useRef<MentionInputHandle>(null);
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
  // A ready agent takes a message even when Settings reads it signed out: the agent's own refusal comes back as the sign-in line (B6).
  const canSend = !answering && agent?.state === "ready";

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
  const tree = at && mentions ? mentionTree(mentions, at.query, menu.folds) : null;
  const enteredSessions = entered && mentions ? mentions.sessions.filter((session) => entered.sessions.includes(session.id)) : [];
  const values = entered ? [`new:${entered.id}`, ...enteredSessions.map((session) => `session:${session.id}`)] : tree ? [...tree.rows, ...tree.found].map((row) => `session:${row.session.id}`).concat(tree.projects.map((project) => `project:${project.id}`)) : [];
  const menuOpen = at !== null && menu.dismissed !== at.start && values.length > 0;
  const active = values.includes(menu.active) ? menu.active : (values[0] ?? "");

  const pick = (mention: DeskMention) => {
    if (!at) return;
    const next = pickMention(draft, at, mention);
    setDraft(next.draft);
    setMenu({ dismissed: null, entered: null, active: "", folds: {} });
    setCaret(next.caret);
    box.current?.place(next.caret);
    box.current?.focus();
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
  const fold = (session: MentionSession, open: boolean) => setMenu({ ...menu, active: `session:${session.id}`, folds: { ...menu.folds, [session.id]: open } });

  // The `@` menu's keys; the composer sends on ↵ when the menu is closed.
  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (event.nativeEvent.isComposing || !menuOpen) return;
    const key = event.key;
    const row = tree?.rows.find((one) => `session:${one.session.id}` === active);
    if (key === "ArrowDown" || key === "ArrowUp") {
      event.preventDefault();
      const index = values.indexOf(active);
      const next = (index + (key === "ArrowDown" ? 1 : values.length - 1)) % values.length;
      setMenu({ ...menu, active: values[next]! });
    } else if (key === "ArrowRight" && row && row.children > 0 && !row.open) {
      // → opens a folded parent and ← folds it again, as the sidebar's tree does (D-84).
      event.preventDefault();
      fold(row.session, true);
    } else if (key === "ArrowLeft" && row && (row.open || row.session.parent !== null)) {
      event.preventDefault();
      if (row.open) fold(row.session, false);
      else setMenu({ ...menu, active: `session:${row.session.parent}` });
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
  const room = useRoomActions(actions);
  const chipSource = useSessionChipSource(room);
  const chatActions = useMemo<DeskChatActions>(() => ({
    onOpen: (job) => room.job(job),
    onAnswer: (job) => room.job(job, true),
    onCancel: (job) => actions.deskJobCancel(job.id),
    onRequeue: (job) => actions.deskJobRequeue(job.id),
    onRetry: (job) => actions.deskJobRequeue(job.id),
    onOutput: room.output,
    onMission: room.mission,
    onApproval: (id, answer, text) => actions.deskApproval(id, answer, text),
    onNoticeAction,
    onCopy: (text) => void actions.copyText(text, "desk sign-in command"),
    onMention: (mention) => {
      if (mention.kind === "session") return room.session({ name: mention.name, mark: mention.mark ?? null, agent_kind: mention.agent_kind ?? null, pane_id: mention.id });
      const rest = useShellStore.getState().rest;
      const project = catalogWorkspaces(rest).find((row) => row.checkouts.some((checkout) => checkout.path === mention.id));
      const checkout = project?.checkouts.find((row) => row.path === mention.id);
      if (project && checkout) actions.openWorkspace(project.device_id, project.id, checkout.id);
    },
    onHanded: (paneId) => room.session({ name: "", mark: null, agent_kind: null, pane_id: paneId }),
  }), [actions, onNoticeAction, room]);
  const place = useUiStore((s) => s.deskRoom);
  const front = activeTab(place.panel);

  if (!desk) return null;
  const working = jobs.filter((job) => job.state === "working").length;
  // The card or chip that opened what shows is marked (B16).
  const shownJob = place.session ? (jobs.find((job) => job.pane_id === place.session)?.id ?? null) : null;
  const shownOutput = front?.kind === "output" ? front.output.id : null;
  const composer = (
    <DeskComposer
      value={draft.text}
      draft={draft}
      inputRef={box}
      label={t("desk.composer.label")}
      placeholder={t("desk.composer.placeholder")}
      answering={answering}
      held={!canSend}
      focused={menuOpen}
      menuOpen={menuOpen}
      menu={menuOpen && tree ? (
        <DeskMentionMenu
          tree={tree}
          searching={at.query !== ""}
          entered={entered ? { project: entered, sessions: enteredSessions } : null}
          active={active}
          onActive={(value) => setMenu({ ...menu, active: value })}
          onSession={pickSession}
          onProject={pickProject}
          onFold={fold}
          onBack={() => setMenu({ ...menu, entered: null, active: "" })}
        />
      ) : null}
      onMenuOpenChange={(open) => {
        if (!open && at) setMenu({ ...menu, dismissed: at.start });
      }}
      onChange={(text, next) => {
        setDraft({ ...draft, text });
        setCaret(next);
      }}
      onCaret={setCaret}
      onKeyDown={onKeyDown}
      onSend={() => send()}
      onStop={actions.deskStop}
    >
      <DeskAgentBar agents={desk.agents} selection={desk.selection} onAgent={(id) => actions.deskSelect(id, null, null)} onModel={(model) => actions.deskSelect(null, model, null)} onEffort={(effort) => actions.deskSelect(null, null, effort)} onAgentSettings={() => actions.openSettings("agents")} />
    </DeskComposer>
  );
  return (
    <SessionChipProvider source={chipSource}>
      <div className="flex min-h-0 min-w-0 flex-1" data-desk-screen="true">
        <DeskRoomView
          actions={actions}
          room={room}
          chat={(
            <DeskChatColumn head={<DeskChatHead onNewConversation={actions.deskNewConversation} />} composer={composer} scrollRef={scroller}>
              {shown.length === 0 && !answering ? (
                <DeskEmpty working={working} asking={jobs.filter((job) => job.state === "asking").length} onExample={(text) => (canSend ? send({ text, mentions: [] }) : setDraft({ text, mentions: [] }))} />
              ) : (
                <DeskChat items={shown} jobs={byId} agents={desk.agents} live={liveText} thinking={answering && liveText === null} startedAt={turn} selectedJob={shownJob} selectedOutput={shownOutput} {...chatActions} />
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
 * every card Desk has, so a chip in the chat or the session panel shows its
 * row's mark, dims once the row is gone, and opens its card under the
 * pointer; a press opens the session beside Desk.
 */
function useSessionChipSource(room: RoomActions): SessionChipSource {
  const rows = useShellStore((s) => allAgents(s.agents, s.rest?.status?.remote));
  const section = useShellStore((s) => s.deskJobs);
  return useMemo(() => {
    const byPane = new Map(rows.map((row) => [row.pane_id, row]));
    const cards = laneCards(section);
    return { facts: (session) => sessionFacts(byPane, cards, session), onOpen: room.session };
  }, [rows, section, room]);
}

/** Every card Desk has: its jobs, their missions' workers, and the other sessions its conversation names. */
function everyCard(): DeskJob[] {
  return laneCards(useShellStore.getState().deskJobs);
}

/** The output a session's own line names, as its card lists it when it does: the View's id, or its address or file. */
function outputOf(job: DeskJob | undefined, output: DeskOutput): DeskOutput {
  return job?.outputs.find((made) => made.id === output.id || (output.url !== null && made.url === output.url) || (output.path !== null && namesView(made, output.path))) ?? output;
}

/** A mission's title as the mission list names it, else as its Conductor's card does. */
function missionTitle(missionId: string): string {
  return useShellStore.getState().rest?.missions?.items.find((one) => one.id === missionId)?.title ?? everyCard().find((job) => job.mission?.id === missionId)?.mission?.title ?? missionId;
}

/** What a press in Desk opens (PRD desk-2-ui B2, B13, B16, B20, B47). */
type RoomActions = {
  /** A card: its session beside Desk; `답하기` gives its input box the keyboard. */
  job: (job: DeskJob, answer?: boolean) => void;
  /** An output: its session beside Desk, the output in front of the tab panel. */
  output: (output: DeskOutput) => void;
  /** A mission chip: its map in the tab panel, beside its Conductor when Desk started it. */
  mission: (missionId: string) => void;
  /** A chip, a letter's name, a handed line: that session beside Desk, by its pane, or by the name its sidebar row carries. */
  session: (session: DeskSessionRef) => void;
};

function useRoomActions(actions: Actions): RoomActions {
  return useMemo(() => {
    const open = (pane: string, answer = false) => actions.openDeskSession(pane, { answer });
    return {
      job: (job, answer = false) => {
        if (job.pane_id) open(job.pane_id, answer);
        else useShellStore.getState().noteDiagnostic("desk job: no pane behind the card pressed");
      },
      output: (output) => {
        const job = everyCard().find((one) => one.outputs.some((made) => made.id === output.id));
        if (job?.pane_id) open(job.pane_id);
        actions.deskOpen({ kind: "output", output: outputOf(job, output) });
      },
      mission: (missionId) => {
        actions.deskOpen({ kind: "mission", mission: missionId, title: missionTitle(missionId) });
      },
      // D-09, B9: a session is its sidebar row's; by its pane, else by the work name the row shows.
      session: (session) => {
        const state = useShellStore.getState();
        const pane = session.pane_id ?? allAgents(state.agents, state.rest?.status?.remote).find((row) => row.identity_label === session.name)?.pane_id;
        if (!pane) return state.noteDiagnostic("desk session: no pane for the session pressed");
        open(pane);
      },
    };
  }, [actions]);
}

/**
 * The app's actions as Desk's parts call them (PRD desk-2-ui D-21, B8,
 * B20, B47): the pane head's chips, the ancestor path, the children chip and
 * the Explorer are the app's own, and only where they open something does
 * Desk differ: a mission or a PR opens in the tab panel, a session beside
 * Desk, a file in the tab panel as the app's file view.
 */
function useDeskPartActions(actions: Actions, room: RoomActions): Actions {
  return useMemo(() => ({
    ...actions,
    openMission: (missionId: string) => room.mission(missionId),
    openSessionPullRequest: (pull: { url: string; number: number }) => actions.deskOpen({ kind: "output", output: { id: `pr:${pull.url}`, kind: "pr", label: `#${pull.number}`, thumb: null, seen: true, pr_state: null, device_id: null, checkout: null, path: null, url: pull.url } }),
    followRelation: (_source: string, target: string) => actions.openDeskSession(target),
    openAgent: (pane: string) => actions.openDeskSession(pane),
    openFile: (path: string, preview: boolean) => openExplorerFile(actions, path, preview),
    openFileBeside: (path: string) => openExplorerFile(actions, path, false),
  }) as Actions, [actions, room]);
}

/** A file the Explorer opened: in the tab panel, under the checkout the Explorer shows (B36). */
function openExplorerFile(actions: Actions, path: string, preview: boolean) {
  const context = explorerContext(useShellStore.getState().rest);
  if (!context.checkout) return useShellStore.getState().noteDiagnostic("desk explorer: no checkout in front");
  actions.deskOpen({ kind: "file", device_id: context.device, root: context.checkout.path, path }, { preview });
}

/** The pane row of `paneId`, as its tab lists it, for the ancestor path (B47). */
function paneRowOf(rest: SnapshotRest | null, paneId: string): PaneRow | null {
  const local = rest?.navigator?.workspaces ?? [];
  const remote = (rest?.status?.remote ?? []).flatMap((row) => row.session?.workspaces ?? []);
  for (const workspace of [...local, ...remote]) {
    for (const checkout of workspace.checkouts) {
      for (const tab of checkout.tabs) {
        const pane = tab.panes.find((one) => one.id === paneId);
        if (pane) return pane as PaneRow;
      }
    }
  }
  return null;
}

/**
 * Desk's room with the session beside it and the tab panel (PRD desk-2-ui
 * B1-B26): Escape closes the tab panel first, then the session panel, each
 * through the shell's Escape owner; closing the tab panel gives the keyboard
 * back to what opened it (B21).
 */
function DeskRoomView({ actions, room, chat }: { actions: Actions; room: RoomActions; chat: React.ReactNode }) {
  const place = useUiStore((s) => s.deskRoom);
  const set = useUiStore((s) => s.setDeskRoom);
  const lane = useShellStore((s) => s.desk?.lane ?? NO_WIDTHS);
  const deskActions = useDeskPartActions(actions, room);
  const panelOpen = place.panel !== null;
  useEscapeLayer(place.session !== null, () => set(closeSession));
  useEscapeLayer(panelOpen, () => set(closePanel));
  // B21: the keyboard goes back to the card or chip that opened the tab panel.
  const opener = useRef<HTMLElement | null>(null);
  useLayoutEffect(() => {
    if (panelOpen) {
      if (!opener.current) opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      return;
    }
    const back = opener.current;
    opener.current = null;
    if (back?.isConnected) back.focus();
  }, [panelOpen]);
  return (
    <DeskRoom
      chat={chat}
      widths={lane}
      session={place.session ? <DeskSessionPanel key={place.session} paneId={place.session} actions={actions} deskActions={deskActions} room={room} /> : null}
      panel={place.panel ? <DeskPanelView actions={actions} deskActions={deskActions} room={room} /> : null}
      onDim={() => set(closePanel)}
      onWidth={actions.deskLaneWidth}
    />
  );
}

const NO_WIDTHS = { view_width: null, session_width: null };

/**
 * The session open beside Desk (PRD desk-2-ui D-01, D-09-D-11, D-20, D-28,
 * B2, B6-B15, B27, B30, B34, B43, B47): its head as the pane head draws it,
 * its conversation read from its pane, the question it waits on as a card,
 * and its input box, or `에이전트 깨우기` in the box's place while it sleeps.
 */
function DeskSessionPanel({ paneId, actions, deskActions, room }: { paneId: string; actions: Actions; deskActions: Actions; room: RoomActions }) {
  const { t } = useInterfaceTranslation();
  const job = useShellStore((s) => (s.deskJobs ? laneCards(s.deskJobs).find((one) => one.pane_id === paneId) ?? null : null));
  const ownJob = useShellStore((s) => s.deskJobs?.jobs.some((one) => one.pane_id === paneId) ?? false);
  const row = useShellStore((s) => agentsByPane(allAgents(s.agents, s.rest?.status?.remote)).get(paneId) ?? null);
  const pane = useShellStore((s) => paneRowOf(s.rest, paneId));
  const machine = useShellStore((s) => (row ? machineName(s.rest?.navigator?.devices, row.device_id, t) : null));
  const panel = useUiStore((s) => s.deskRoom.panel);
  const answer = useUiStore((s) => s.deskRoom.answer);
  const filesChord = useShellStore((s) => commandLabel("toggle_lane_explorer", s.rest?.ui_state)) || null;
  const terminalChord = useShellStore((s) => commandLabel("desk_terminal", s.rest?.ui_state)) || null;
  const sleeping = job?.state === "sleeping";
  const asleep = useSleepingOf(sleeping ? job : null, row);
  const front = activeTab(panel);
  // Reading the session's card reads its row, as reading it in the chat does.
  const unread = ownJob && job && !job.read ? job.id : null;
  useEffect(() => {
    if (unread) actions.deskJobRead(unread);
  }, [actions, unread]);
  const write = () => useUiStore.getState().setDeskRoom((now) => openSession(now, paneId, { answer: true }));
  return (
    <ThreadPanel
      job={job}
      paneId={paneId}
      actions={actions}
      startedByDesk={ownJob}
      selectedOutput={front?.kind === "output" ? front.output.id : null}
      answer={answer}
      head={job ? (
        <SessionHead
          paneId={paneId}
          job={job}
          row={row}
          pane={pane}
          machine={machine}
          disconnected={job.disconnected}
          sleeping={sleeping}
          selectedOutput={front?.kind === "output" ? front.output.id : null}
          explorerOpen={panel?.explorer === true}
          filesChord={filesChord}
          terminalChord={terminalChord}
          deskActions={deskActions}
          onOutput={(output) => actions.deskOpen({ kind: "output", output })}
          onFiles={actions.deskToggleExplorer}
          onTerminal={actions.deskTerminal}
          onClose={() => useUiStore.getState().setDeskRoom(closeSession)}
        />
      ) : null}
      ask={job?.state === "asking" ? <DeskSessionAsk paneId={paneId} deviceId={row?.device_id ?? null} job={job} row={row} actions={actions} onWrite={write} /> : null}
      foot={sleeping ? <SessionSleepingFoot title={row?.identity_label ?? job.title} session={asleep} onWake={(id) => wake(actions, id, job.title, paneId)} /> : null}
      onTerminal={actions.deskTerminal}
      onOutput={(output) => actions.deskOpen({ kind: "output", output: outputOf(job ?? undefined, output) })}
      onMission={room.mission}
      onSession={room.session}
    />
  );
}

/**
 * What the session beside Desk asks, read from its screen with `pane_ask`
 * each time its ask changes (PRD desk-2-ui B14), and answered with
 * `pane_choice` as the phone answers it: a choice is marked, `답하기` sends
 * it once, and one that may have landed or that the screen no longer shows
 * is answered in the terminal from then on; a choice refused stale reads
 * the card again. Until the read lands, the card
 * says what its row or card says, with `직접 쓰기` only.
 */
function DeskSessionAsk({ paneId, deviceId, job, row, actions, onWrite }: { paneId: string; deviceId: string | null; job: DeskJob; row: AgentRow | null; actions: Actions; onWrite: () => void }) {
  const { t } = useInterfaceTranslation();
  // The daemon does not push an ask: it is read when the panel opens and again whenever the agent's row moves (its change time or group).
  const changed = row?.changed_at_unix_ms ?? null;
  const group = row?.group ?? null;
  useEffect(() => {
    if (deviceId) actions.askPane(deviceId, paneId);
  }, [actions, deviceId, paneId, changed, group, job.changed_at_unix_ms]);
  const read = useShellStore((s) => (s.paneAsk && s.paneAsk.pane_id === paneId && s.paneAsk.device_id === deviceId ? s.paneAsk : null));
  const [marked, setMarked] = useState<{ ask: string; id: string } | null>(null);
  const [sent, setSent] = useState<{ ask: string; choice: string; request: string | null } | null>(null);
  const result = useShellStore((s) => (sent?.request && s.paneChoice?.request_id === sent.request ? s.paneChoice : null));
  const ask = read?.ask ?? null;
  const waited = useElapsed(ask?.since_ms ?? null);
  // A choice refused stale means the agent asks something else now: the card is read again.
  const stale = result?.ok === false && result.reason === "stale" ? result.request_id : null;
  useEffect(() => {
    if (stale && deviceId) actions.askPane(deviceId, paneId);
  }, [actions, deviceId, paneId, stale]);
  if (!ask || !deviceId) {
    const question = sessionQuestion(job, row);
    return question ? <SessionQuestionCard question={question} onWrite={onWrite} /> : null;
  }
  const identity = askIdentity({ device_id: deviceId, pane_id: paneId }, ask);
  const mine = sent?.ask === identity ? sent : null;
  const answer = !mine ? null : mine.request === null || (result && !result.ok) ? "failed" : result ? "sent" : "sending";
  const reason = mine?.request === null ? "offline" : result && !result.ok ? result.reason : null;
  const inTerminal = answer === "failed" && (reason === "uncertain" || reason === "screen_differs" || reason === "ask_not_shown");
  return (
    <SessionAskCard
      ask={ask}
      disabled={false}
      waited={waited}
      picked={marked?.ask === identity ? marked.id : (mine?.choice ?? null)}
      answer={answer}
      error={answer === "failed" ? noticeText(t, inputFailure(reason)) : null}
      inTerminal={inTerminal}
      onPick={(id) => setMarked({ ask: identity, id })}
      onAnswer={(choice) => {
        // An answer sent, or one that may have landed, is never sent again from here: a second send would be a second press.
        if (answer === "sending" || answer === "sent" || inTerminal) return;
        setSent({ ask: identity, choice, request: actions.answerPane(deviceId, paneId, ask, choice) });
      }}
      onTerminal={actions.deskTerminal}
      onWrite={onWrite}
    />
  );
}

/** The sidebar's sleeping row of a session asleep beside Desk: its work name in its checkout (B43). */
function useSleepingOf(job: DeskJob | null, row: AgentRow | null) {
  return useShellStore((s) => (job ? s.rest?.navigator?.sleeping_sessions?.find((one) => one.identity_label === (row?.identity_label ?? job.title)) ?? null : null));
}

/**
 * `에이전트 깨우기` (B43): the session wakes in a new pane, which takes the
 * place of the one that slept beside Desk once the sidebar lists it.
 */
function wake(actions: Actions, sleepId: string, title: string, oldPane: string) {
  actions.wakeSleepingSession(sleepId);
  const follow = useShellStore.subscribe((state) => {
    const woke = allAgents(state.agents, state.rest?.status?.remote).find((one) => one.identity_label === title && one.pane_id !== oldPane);
    if (!woke) return;
    follow();
    window.clearTimeout(limit);
    if (useUiStore.getState().deskRoom.session === oldPane) useUiStore.getState().setDeskRoom((now) => openSession(now, woke.pane_id));
  });
  // A wake that never lists the session stops being followed.
  const limit = window.setTimeout(follow, WAKE_FOLLOW_MS);
}

/** How long a wake is followed before Desk stops looking for its new pane. */
const WAKE_FOLLOW_MS = 60_000;

/**
 * The tab panel (PRD desk-2-ui D-05, D-08, D-22, D-25, B16-B21, B35, B36,
 * B40): its tabs, the Explorer's column when `파일` opened it, and the tab in
 * front drawn with the app's own viewer: the file view, Desk's output
 * viewer, the mission map.
 */
function DeskPanelView({ actions, deskActions, room }: { actions: Actions; deskActions: Actions; room: RoomActions }) {
  const { t } = useInterfaceTranslation();
  const panel = useUiStore((s) => s.deskRoom.panel);
  const set = useUiStore((s) => s.setDeskRoom);
  const rest = useShellStore((s) => s.rest);
  if (!panel) return null;
  const projects = catalogWorkspaces(rest);
  const projectOf = (tab: Extract<DeskTab, { kind: "file" }>) => projects.find((project) => project.checkouts.some((checkout) => checkout.path === tab.root))?.label ?? tab.root;
  const names = tabNames(panel.tabs, projectOf);
  const front = activeTab(panel);
  const tabs: PanelTab[] = panel.tabs.map((tab) => {
    const key = tabKey(tab);
    const name = names.get(key)!;
    return {
      id: key,
      label: tab.kind === "mission" ? t("desk.lane.missionMap", { title: name.name }) : name.name,
      project: name.project,
      preview: panel.preview === key,
      icon: tab.kind === "output" ? <OutputMark output={tab.output} /> : tab.kind === "file" ? <FileMark icon={fileIcon(tab.path)} /> : <MapIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />,
    };
  });
  let body: React.ReactNode;
  if (front?.kind === "output") body = <DeskOutputView key={tabKey(front)} output={front.output} viewKey={tabKey(front)} actions={actions} />;
  else if (front?.kind === "mission") body = <MissionMap missionId={front.mission} actions={deskActions} onNode={(node) => (node.pane_id ? room.session({ name: "", mark: null, agent_kind: null, pane_id: node.pane_id }) : undefined)} />;
  else if (front?.kind === "file") body = <DeskFileView key={tabKey(front)} tab={front} actions={actions} />;
  else body = <p className="m-auto px-md text-caption text-muted-foreground" data-desk-panel-empty="true">{t("desk.room.explorerEmpty")}</p>;
  return (
    <DeskTabPanel
      tabs={tabs}
      active={front ? tabKey(front) : null}
      tool={panel.explorer ? <ExplorerTree actions={deskActions} /> : null}
      onTab={(key) => {
        const tab = panel.tabs.find((one) => tabKey(one) === key);
        // A file's tab brings its checkout's file view in front again (`deskOpen` keeps its place).
        if (tab?.kind === "file") actions.deskOpen(tab, { preview: panel.preview === key });
        else set((now) => showTab(now, key));
      }}
      onKeep={(key) => {
        const tab = panel.tabs.find((one) => tabKey(one) === key);
        if (tab?.kind === "file" && panel.preview === key) actions.deskOpen(tab);
        else set((now) => keepTab(now, key));
      }}
      onCloseTab={(key) => set((now) => closeTab(now, key))}
      onClose={() => set(closePanel)}
    >
      {body}
    </DeskTabPanel>
  );
}

/**
 * A file's tab (PRD desk-2-ui D-22, B19, B36): the app's own file view of
 * the display the core placed for it in its checkout, with editing and ⌘S;
 * while the core has not placed it yet, or the checkout is not in front, the
 * file view's own loading line.
 */
function DeskFileView({ tab, actions }: { tab: Extract<DeskTab, { kind: "file" }>; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const view = useShellStore((s) => workspaceViewOf(s.rest));
  const front = useShellStore((s) => frontCheckout(s.rest)?.path ?? null);
  const node = useShellStore((s) => localDeviceId(s.rest));
  const here = view !== null && front === tab.root && (view.device_id === tab.device_id || (tab.device_id === node && !view.device_id));
  const display = here && view.layout ? areasOf(view.layout.root).flatMap((area) => area.displays).find((one) => one.kind === "file" && one.path === tab.path) ?? null : null;
  if (!view || !display) return <p role="status" className="m-auto text-caption text-muted-foreground" data-desk-file-loading={tab.path}>{t("common.loading")}</p>;
  const workspace = { device_id: view.device_id, path: view.path };
  return <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-desk-file={tab.path}><DisplayEditor display={display} workspace={workspace} placeKey={placeKey(workspaceKey(workspace), display)} actions={actions} /></div>;
}

/**
 * At the head of an Agent tab Desk sent the operator to (PRD desk-2-ui B7):
 * `나도 참여 중` once the operator typed into a Desk job's, and `Desk로`,
 * which ⌘J also presses, back to Desk with this session beside it.
 */
export function PaneDeskReturn({ paneId, actions }: { paneId: string; actions: Actions }) {
  const job = useShellStore((s) => s.deskJobs?.jobs.find((one) => one.pane_id === paneId) ?? s.deskJobs?.workers.find((one) => one.pane_id === paneId) ?? null);
  const returns = useUiStore((s) => s.deskReturn === paneId);
  const chord = useShellStore((s) => commandLabel("desk_terminal", s.rest?.ui_state)) || null;
  const on = useShellStore((s) => deskOn(s.desk));
  if ((!job && !returns) || !on) return null;
  return <DeskPaneReturn joined={job?.joined ?? false} chord={chord} onDesk={() => actions.openDeskSession(paneId)} />;
}
