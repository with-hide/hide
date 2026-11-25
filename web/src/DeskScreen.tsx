// Desk's room on screen (PRD session-docs-and-assistant, Desk 1차; PRD
// desk-2-ui): the `desk`, `desk_live`, `desk_jobs` and `desk_mentions`
// sections drawn with the Desk parts, each press sent as one `desk_*` event,
// and at the chat's right the session opened beside Desk with the tab panel
// floating at its left edge (`DeskRoom`). The core decides every state; this
// screen keeps what the operator is writing, where the `@` menu is, and the
// room's window state (`deskRoom.ts`), which the core keeps only the widths of.

import { MapIcon, XIcon } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type RefObject } from "react";
import type { Actions } from "./actions";
import { deskOn, laneCards, sessionFacts, type DeskItem, type DeskJob, type DeskMention, type DeskOutput, type DeskQueuedMessage, type DeskSessionRef } from "./desk";
import { activeTab, closePanel, closeSide, closeTab, fileName, keepTab, openSession, outputNamed, showTab, tabKey, tabNames, type DeskTab } from "./deskRoom";
import { DeskOutputView } from "./DeskOutputView";
import { AreaEmpty } from "./AreaEmpty";
import { DisplayEditor, DocumentKeeper } from "./Editor";
import { Button } from "./components/ui/button";
import { ExplorerTree, type ExplorerScope } from "./ExplorerTree";
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
import { keepSent, mentionAt, mentionTree, outgoing, pickMention, useComposerTray } from "./deskComposer";
import { refusalText, stageDeskFiles } from "./attachments";
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
import { composingKey } from "./keys";
import { machineName } from "./screenMachine";
import { catalogWorkspaces, checkoutAtRoot, checkoutOfPane, localDeviceId, paneRowOf, type AgentRow, type SleepingSession, type ViewDisplaySnapshot } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { windowIsActive } from "./viewFocus";

const NO_JOBS: DeskJob[] = [];

/** Settings › Labs › Desk (B4, B19, B21). */
export function DeskSettingsPage({ actions }: { actions: Actions }) {
  const desk = useShellStore((s) => s.desk);
  if (!desk) return null;
  return <DeskSettingsTab settings={desk.settings} agents={desk.agents} onChange={(settings) => actions.deskSettings(settings)} />;
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
  // The words a send carried, held until the conversation shows them, so a refused send keeps them in the box; its files' request, when it had files.
  const pending = useRef<{ text: string; after: number; request: string | null; tiles: string[]; queued: ReadonlySet<number> } | null>(null);
  const tray = useComposerTray();
  const [notice, setNotice] = useState<string | null>(null);
  const refusal = useShellStore((s) => s.composerRefusal);

  const byId = useMemo(() => Object.fromEntries(jobs.map((job) => [job.id, job])), [jobs]);
  const items = desk?.conversation.items;
  // A card past the newest 64 the section carries is not drawn; its line in the conversation goes with it.
  const shown = useMemo(() => (items ?? []).filter((item) => item.kind !== "job" || byId[item.job_id] !== undefined), [items, byId]);
  const turn = live?.started_at_ms ?? null;
  const answering = turn !== null;
  const liveText = live?.text ? live.text : null;
  const agent = desk?.selection.agent ? desk.agents.find((one) => one.id === desk.selection.agent) : undefined;
  // A ready agent takes a message even when Settings reads it signed out: the agent's own refusal comes back as the sign-in line (B6).
  // While Desk answers a message waits for its turn (it shows in the conversation as 대기 중); only an agent not ready holds it.
  // A message the agent could not take (its sign-in, a session that did not open, a file that is gone) comes back as the
  // first waiting one with its files, so the box never gets it again (B6). The core refuses one more when the line is
  // full: then the words and files stay in the box.
  const queued = desk?.conversation.queued ?? NO_QUEUED;
  const queuedFull = desk?.conversation.queued_full ?? false;
  const queuedHeld = desk?.conversation.queued_held ?? false;
  const canSend = agent?.state === "ready" && !queuedFull;

  const lastUserId = (list: readonly DeskItem[]) => list.reduce((last, item) => (item.kind === "user" ? item.id : last), -1);
  useEffect(() => {
    const sent = pending.current;
    if (!sent || !items) return;
    const shown = items.some((item) => item.kind === "user" && item.id > sent.after && item.text === sent.text) || queued.some((one) => !sent.queued.has(one.id) && one.text === sent.text);
    if (shown) {
      pending.current = null;
      if (outgoing(useUiStore.getState().deskDraft).text === sent.text) setDraft({ text: "", mentions: [] });
      // Only the files this send carried: one dropped since stays for the next.
      sent.tiles.forEach(tray.remove);
    }
  }, [items, queued, setDraft]);
  // hided refused the files before Desk's agent saw the message: the words and the files stay, and the line says why.
  useEffect(() => {
    const sent = pending.current;
    if (!refusal || !sent?.request || refusal.request_id !== sent.request) return;
    pending.current = null;
    setNotice(refusalText(refusal.reason, t));
  }, [refusal, t]);

  useLayoutEffect(() => {
    const box = scroller.current;
    if (box && stick.current) box.scrollTop = box.scrollHeight;
  }, [shown, liveText, answering, queued]);
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
    const files = tray.outgoing();
    if (!files || (!out.text && files.length === 0)) return;
    const stagedOrRefused = files.length > 0 ? stageDeskFiles(files.map(({ name, bytes }) => ({ name, bytes }))) : null;
    if (stagedOrRefused && "refused" in stagedOrRefused) return setNotice(refusalText(stagedOrRefused.refused, t));
    if (files.length > 0 && !stagedOrRefused) return setNotice(refusalText("forward_failed", t));
    const staged = stagedOrRefused;
    // The pictures this window sent stay drawn on their bubble while the page lives.
    if (staged) files.forEach((file, index) => keepSent(staged.stages[index]!, file.name, file.image));
    setNotice(null);
    pending.current = { text: out.text, after: lastUserId(items), request: staged?.request_id ?? null, tiles: files.map((file) => file.id), queued: new Set(queued.map((one) => one.id)) };
    stick.current = true;
    actions.deskSend(out.text, out.mentions, staged);
  };

  // The `@` menu: open while the caret ends an `@` word Esc did not close; when nothing matches it says so in one row.
  const at = mentionAt(draft.text, caret);
  const entered = menu.entered !== null ? mentions?.projects.find((project) => project.id === menu.entered) ?? null : null;
  const tree = at && mentions ? mentionTree(mentions, at.query, menu.folds) : null;
  const enteredSessions = entered && mentions ? mentions.sessions.filter((session) => entered.sessions.includes(session.id)) : [];
  const values = entered ? [`new:${entered.id}`, ...enteredSessions.map((session) => `session:${session.id}`)] : tree ? [...tree.rows, ...tree.found].map((row) => `session:${row.session.id}`).concat(tree.projects.map((project) => `project:${project.id}`)) : [];
  const menuOpen = at !== null && menu.dismissed !== at.start && (values.length > 0 || tree !== null);
  const menuEmpty = menuOpen && values.length === 0;
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
    if (composingKey(event) || !menuOpen) return;
    const key = event.key;
    // With nothing to choose the menu has no keys of its own but Esc, which closes it; ↵ sends the words.
    if (menuEmpty) {
      if (key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        setMenu({ ...menu, dismissed: at!.start });
      }
      return;
    }
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
        if (last?.kind === "user") actions.deskSend(last.text, last.mentions, null, last.id);
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
    onQueuedRemove: (id) => actions.deskQueuedRemove(id),
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
      tray={tray}
      notice={queuedFull ? t("desk.queued.full") : notice}
      answering={answering}
      held={!canSend}
      focused={menuOpen}
      menuOpen={menuOpen}
      menuEmpty={menuEmpty}
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
      onStop={() => actions.deskStop()}
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
            <DeskChatColumn head={<DeskChatHead onNewConversation={() => actions.deskNewConversation()} />} composer={composer} scrollRef={scroller}>
              {shown.length === 0 && !answering && queued.length === 0 ? (
                <DeskEmpty working={working} asking={jobs.filter((job) => job.state === "asking").length} onExample={(text) => (canSend ? send({ text, mentions: [] }) : setDraft({ text, mentions: [] }))} />
              ) : (
                <DeskChat items={shown} jobs={byId} agents={desk.agents} queued={queued} held={queuedHeld} live={liveText} thinking={answering && liveText === null} startedAt={turn} selectedJob={shownJob} selectedOutput={shownOutput} {...chatActions} />
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

/** The output a session's own line names, as `outputNamed` resolves it, on this machine's node. */
function outputOf(job: DeskJob | undefined, output: DeskOutput, place?: { device: string; checkout: string } | null): DeskOutput {
  return outputNamed(job?.outputs ?? [], output, place ?? null, localDeviceId(useShellStore.getState().rest));
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
function useDeskPartActions(actions: Actions, room: RoomActions, scope: ExplorerScope | null): Actions {
  return useMemo(() => {
    // Every other action keeps the app's own object as `this`, so a method that calls another reaches the app's, never Desk's.
    const bound = Object.fromEntries(Object.entries(actions).map(([key, value]) => [key, typeof value === "function" ? value.bind(actions) : value])) as Actions;
    const desk: Actions = {
      ...bound,
      ...(scope ? actions.explorerIn(scope.device, scope.checkout.path) : {}),
      openMission: (missionId: string) => room.mission(missionId),
      openSessionPullRequest: (pull: Parameters<Actions["openSessionPullRequest"]>[0]) => openPullRequest(actions, pull.url, `#${pull.number}`),
      followRelation: (_source: string, target: string) => actions.openDeskSession(target),
      openAgent: (pane: string) => actions.openDeskSession(pane),
      openFile: (path: string, preview: boolean) => openExplorerFile(actions, scope, path, { preview }),
      openFileBeside: (path: string) => openExplorerFile(actions, scope, path, { beside: true }),
    };
    return desk;
  }, [actions, room, scope]);
}

/** A pull request in the tab panel: the output its card lists for it when one does, so the same PR is one tab (m6). */
function openPullRequest(actions: Actions, url: string, label: string) {
  const listed = everyCard().flatMap((job) => job.outputs).find((output) => output.kind === "pr" && output.url === url);
  actions.deskOpen({ kind: "output", output: listed ?? { id: `pr:${url}`, kind: "pr", label, thumb: null, seen: true, pr_state: null, device_id: null, checkout: null, path: null, url } });
}

/** A file Desk's Explorer opened: in the tab panel, under the session's checkout it lists (B36, B37). */
function openExplorerFile(actions: Actions, scope: ExplorerScope | null, path: string, options: { preview?: boolean; beside?: boolean }) {
  if (!scope) return useShellStore.getState().noteDiagnostic("desk explorer: no session checkout");
  actions.deskOpen({ kind: "file", device_id: scope.device, root: scope.checkout.path, path }, options);
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
  // The session's checkout, which Desk's Explorer lists whatever is in front (B35).
  const scopeCheckout = useShellStore((s) => (place.session ? (checkoutOfPane(s.rest, place.session)?.checkout ?? null) : null));
  const scopeDevice = useShellStore((s) => (place.session ? (checkoutOfPane(s.rest, place.session)?.device ?? null) : null));
  const scope = useMemo<ExplorerScope | null>(() => (scopeCheckout && scopeDevice ? { device: scopeDevice, checkout: scopeCheckout } : null), [scopeCheckout, scopeDevice]);
  const deskActions = useDeskPartActions(actions, room, scope);
  const panelOpen = place.panel !== null;
  // The session beside Desk covers nothing, so it is no Escape layer: Escape closes it from the shell's Escape owner once no
  // layer is open (keyboard.ts). The tab panel dims the rest and is one.
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
      panel={place.panel ? <DeskPanelView actions={actions} deskActions={deskActions} room={room} scope={scope} /> : null}
      onDim={() => set(closePanel)}
      onWidth={(panel, width) => actions.deskLaneWidth(panel, width)}
    />
  );
}

const NO_QUEUED: readonly DeskQueuedMessage[] = [];
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
  const checkoutPath = useShellStore((s) => checkoutOfPane(s.rest, paneId)?.checkout.path ?? null);
  const machine = useShellStore((s) => (row ? machineName(s.rest?.navigator?.devices, row.device_id, t) : null));
  const panel = useUiStore((s) => s.deskRoom.panel);
  const answer = useUiStore((s) => s.deskRoom.answer);
  const filesChord = useShellStore((s) => commandLabel("toggle_lane_explorer", s.rest?.ui_state)) || null;
  const terminalChord = useShellStore((s) => commandLabel("desk_terminal", s.rest?.ui_state)) || null;
  // Who the session is, kept while its row is listed: a session that sleeps or whose pane closes loses its row and its
  // card, and the panel still names it and finds it among the sleeping sessions (B6, B43).
  const known = useRef<SessionIdentity | null>(null);
  if (row && checkoutPath) known.current = { title: row.identity_label, node: row.device_id, checkout: checkoutPath };
  const identity = known.current;
  const asleep = useShellStore((s) => (identity ? sleepingOf(s.rest?.navigator?.sleeping_sessions, identity) : null));
  // Most agents sleep in their own pane, which stays (PRD agent-sleep) and wakes there; an agent whose sleep closes
  // its pane (Pi's) is found among the sleeping sessions and wakes in a new one (B43).
  const inPane = row?.sleep ?? null;
  const sleeping = inPane !== null || job?.state === "sleeping" || (!row && asleep !== null);
  const title = row?.identity_label ?? job?.title ?? identity?.title ?? "";
  const [waking, setWaking] = useState<SessionIdentity | null>(null);
  useWakeFollow(waking, paneId, () => setWaking(null));
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
      head={
        <SessionHead
          paneId={paneId}
          title={title}
          job={job}
          row={row}
          pane={pane}
          machine={machine}
          disconnected={job?.disconnected ?? false}
          sleeping={sleeping}
          selectedOutput={front?.kind === "output" ? front.output.id : null}
          explorerOpen={panel?.explorer === true}
          filesChord={filesChord}
          terminalChord={terminalChord}
          deskActions={deskActions}
          onOutput={(output) => actions.deskOpen({ kind: "output", output })}
          onFiles={() => actions.deskToggleExplorer()}
          onTerminal={() => actions.deskTerminal()}
          onClose={() => actions.deskCloseSession()}
        />
      }
      ask={job?.state === "asking" ? <DeskSessionAsk paneId={paneId} deviceId={row?.device_id ?? null} job={job} row={row} actions={actions} onWrite={write} /> : null}
      foot={sleeping ? (
        <SessionSleepingFoot
          title={title}
          sleep={inPane}
          state={asleep?.phase ?? "unknown"}
          onFresh={() => actions.wakeAgent(paneId, true)}
          onWake={
            inPane
              ? inPane.state === "waking" ? null : () => actions.wakeAgent(paneId)
              : asleep?.wake_available
                ? () => {
                    actions.wakeSleepingSession(asleep.sleep_id);
                    setWaking(identity);
                  }
                : null
          }
        />
      ) : null}
      onTerminal={() => actions.deskTerminal()}
      onOutput={(output) => actions.deskOpen({ kind: "output", output: outputOf(job ?? undefined, output, row && checkoutPath ? { device: row.device_id, checkout: checkoutPath } : null) })}
      onMission={room.mission}
      onSession={room.session}
    />
  );
}

/** A session by what outlives its pane: its work name, its machine and its checkout (O3: a name alone is not one session). */
type SessionIdentity = { title: string; node: string; checkout: string };

function sleepingOf(sessions: SleepingSession[] | undefined, identity: SessionIdentity): SleepingSession | null {
  return sessions?.find((one) => one.node_id === identity.node && one.checkout_path === identity.checkout && one.identity_label === identity.title) ?? null;
}

/**
 * `에이전트 깨우기` (B43): the session wakes in a new pane, which takes the
 * place of the one that slept beside Desk once the sidebar lists it on the
 * same machine and checkout. The follow ends with the panel, with the next
 * press, or after `WAKE_FOLLOW_MS`.
 */
function useWakeFollow(waking: SessionIdentity | null, oldPane: string, done: () => void) {
  const end = useRef(done);
  end.current = done;
  useEffect(() => {
    if (!waking) return undefined;
    const check = (state: ReturnType<typeof useShellStore.getState>) => {
      const woke = allAgents(state.agents, state.rest?.status?.remote).find(
        (one) => one.pane_id !== oldPane && one.device_id === waking.node && one.identity_label === waking.title && checkoutOfPane(state.rest, one.pane_id)?.checkout.path === waking.checkout,
      );
      if (!woke) return;
      end.current();
      if (useUiStore.getState().deskRoom.session === oldPane) useUiStore.getState().setDeskRoom((now) => openSession(now, woke.pane_id));
    };
    const unsubscribe = useShellStore.subscribe(check);
    const limit = window.setTimeout(() => end.current(), WAKE_FOLLOW_MS);
    return () => {
      unsubscribe();
      window.clearTimeout(limit);
    };
  }, [waking, oldPane]);
}

/** How long a wake is followed before Desk stops looking for its new pane. */
const WAKE_FOLLOW_MS = 60_000;

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
  const [sent, setSent] = useState<{ ask: string; choice: string; request: string | null; dropped?: boolean } | null>(null);
  const result = useShellStore((s) => (sent?.request && s.paneChoice?.request_id === sent.request ? s.paneChoice : null));
  // A connection lost before the answer's result came back leaves no result to wait for: the choice may have landed, so it
  // is answered in the terminal from then on, as a result that says `uncertain` is (review m3).
  const live = useShellStore((s) => s.connection === "live");
  useEffect(() => {
    if (!live) setSent((now) => (now && now.request !== null && !now.dropped ? { ...now, dropped: true } : now));
  }, [live]);
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
  const lost = mine?.dropped === true && !result;
  const answer = !mine ? null : mine.request === null || lost || (result && !result.ok) ? "failed" : result ? "sent" : "sending";
  const reason = mine?.request === null ? "offline" : lost ? "uncertain" : result && !result.ok ? result.reason : null;
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
      onTerminal={() => actions.deskTerminal()}
      onWrite={onWrite}
    />
  );
}

/** The sidebar's sleeping row of a session asleep beside Desk: its work name in its checkout (B43). */

/**
 * The tab panel (PRD desk-2-ui D-05, D-08, D-22, D-25, B16-B21, B35, B36,
 * B40): its tabs, the Explorer's column when `파일` opened it, and the tab in
 * front drawn with the app's own viewer: the file view, Desk's output
 * viewer, the mission map.
 */
function DeskPanelView({ actions, deskActions, room, scope }: { actions: Actions; deskActions: Actions; room: RoomActions; scope: ExplorerScope | null }) {
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
  let keepers: React.ReactNode = null;
  if (front?.kind === "output") body = <DeskOutputView key={tabKey(front)} output={front.output} viewKey={tabKey(front)} actions={actions} />;
  else if (front?.kind === "mission") body = <MissionMap missionId={front.mission} actions={deskActions} onNode={(node) => (node.pane_id ? room.session({ name: "", mark: null, agent_kind: null, pane_id: node.pane_id }) : undefined)} />;
  else if (front?.kind === "file") {
    const beside = panel.beside ? panel.tabs.find((one) => tabKey(one) === panel.beside) : undefined;
    const shown = beside?.kind === "file" ? [front, beside] : [front];
    keepers = <DeskDocumentKeepers files={shown} actions={deskActions} />;
    body = beside?.kind === "file" ? (
      <div className="flex min-h-0 min-w-0 flex-1" data-desk-file-split="true">
        <DeskFileView key={`front:${tabKey(front)}`} tab={front} side="front" actions={deskActions} onClose={() => set((now) => closeSide(now, "front"))} />
        <div aria-hidden="true" className="w-(--size-hairline) shrink-0 bg-border" />
        <DeskFileView key={`beside:${tabKey(beside)}`} tab={beside} side="beside" actions={deskActions} onClose={() => set((now) => closeSide(now, "beside"))} />
      </div>
    ) : (
      <DeskFileView key={tabKey(front)} tab={front} side="front" actions={deskActions} />
    );
  }
  else body = <p className="m-auto px-md text-caption text-muted-foreground" data-desk-panel-empty="true">{t("desk.room.explorerEmpty")}</p>;
  return (
    <DeskTabPanel
      tabs={tabs}
      active={front ? tabKey(front) : null}
      tool={panel.explorer && scope ? <ExplorerTree actions={deskActions} scope={scope} /> : null}
      onTab={(key) => set((now) => showTab(now, key))}
      onKeep={(key) => set((now) => keepTab(now, key))}
      onCloseTab={(key) => set((now) => closeTab(now, key))}
      onClose={() => set(closePanel)}
    >
      {keepers}
      {body}
    </DeskTabPanel>
  );
}

type FileTab = Extract<DeskTab, { kind: "file" }>;

/** The document the core holds for a Desk file tab: the editor tab of its checkout's file, once the core has opened it. */
function heldDocument(state: ReturnType<typeof useShellStore.getState>, tab: FileTab) {
  const checkoutId = checkoutAtRoot(state.rest, tab.device_id, tab.root)?.checkout.id ?? null;
  return state.editor?.tabs.find((one) => one.kind === "file" && one.checkout_id === checkoutId && one.path === tab.path) ?? null;
}

/**
 * One keeper per document the panel shows, however many of its sides show it, as a Workspace's View mounts one
 * (`ViewAreas.tsx`): its autosave, the save of a draft left behind, and the stored draft's restore and its removal once
 * a save lands, which without it stayed in the browser and came back as an unsaved draft after a reload.
 */
function DeskDocumentKeepers({ files, actions }: { files: FileTab[]; actions: Actions }) {
  // One string, so the selector reads the same value until a held document changes.
  const held = useShellStore((s) => [...new Set(files.map((tab) => heldDocument(s, tab)?.id).filter((id) => id !== undefined))].join("\n"));
  return held ? held.split("\n").map((tabId) => <DocumentKeeper key={tabId} tabId={tabId} actions={actions} />) : null;
}

/**
 * A file's tab (PRD desk-2-ui D-22, B19, B36, B37): the app's own file view
 * of the document the core holds for Desk (`desk_file`), with editing and ⌘S,
 * named from its own checkout whatever is in front; the file view's own
 * loading line until the core has read it. With a second view beside it
 * (⌘↵), each side has its own `✕`.
 */
function DeskFileView({ tab, side, actions, onClose }: { tab: Extract<DeskTab, { kind: "file" }>; side: "front" | "beside"; actions: Actions; onClose?: () => void }) {
  const { t } = useInterfaceTranslation();
  // Plain ids, so the selectors read the same value until the catalog moves.
  const workspaceId = useShellStore((s) => checkoutAtRoot(s.rest, tab.device_id, tab.root)?.workspace ?? null);
  const checkoutId = useShellStore((s) => checkoutAtRoot(s.rest, tab.device_id, tab.root)?.checkout.id ?? null);
  const held = useShellStore((s) => heldDocument(s, tab));
  const key = `desk:${side}:${tabKey(tab)}`;
  const display = useMemo<ViewDisplaySnapshot>(
    () => ({ id: key, tab_id: held?.id ?? null, path: tab.path, label: fileName(tab.path), kind: "file", committed: null, preview: false, state: "open", reason: null }),
    [key, held, tab.path],
  );
  const workspace = useMemo(() => ({ device_id: tab.device_id, path: tab.root }), [tab.device_id, tab.root]);
  const close = onClose ? <Button variant="ghost" size="icon-xs" className="shrink-0 text-muted-foreground" aria-label={t("desk.room.closeSide", { name: fileName(tab.path) })} data-desk-file-close={side} onClick={onClose}><XIcon /></Button> : null;
  let body: React.ReactNode;
  // B28: a file Desk cannot hold or read says why, never a view left loading.
  if (!workspaceId || !checkoutId) body = <AreaEmpty state="view-unavailable" text={t("desk.room.fileNotListed", { path: tab.path })} />;
  else if (held?.unavailable_reason) {
    const file = { workspace_id: workspaceId, checkout_id: checkoutId, device_id: tab.device_id, path: tab.path };
    body = (
      <AreaEmpty state="view-unavailable" text={t("documents.view.unavailableReason", { path: tab.path, reason: held.unavailable_reason })}>
        <Button variant="ghost" onClick={() => actions.deskFileRetry(file)} data-desk-file-retry={tab.path}>{t("common.retry")}</Button>
      </AreaEmpty>
    );
  } else if (held) body = <DisplayEditor display={display} workspace={workspace} placeKey={key} actions={actions} root={tab.root} trailing={close} />;
  else body = <p role="status" className="m-auto text-caption text-muted-foreground" data-desk-file-loading={tab.path}>{t("common.loading")}</p>;
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-desk-file={tab.path} data-desk-file-side={side}>
      {body}
    </div>
  );
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
