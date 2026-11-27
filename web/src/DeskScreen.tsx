// Desk's room on screen (PRD session-docs-and-assistant, Desk 1차; PRD
// desk-2-ui): the `desk`, `desk_live`, `desk_jobs` and `desk_mentions`
// sections drawn with the Desk parts, each press sent as one `desk_*` event,
// and at the chat's right the session opened beside Desk with Desk's own desk
// floating at its left edge (`DeskRoom`, PRD desk-own-desk). The core decides
// every state, the desk's tabs included; this screen keeps what the operator
// is writing, where the `@` menu is, and the room's window state
// (`deskRoom.ts`), which the core keeps only the widths of.

import { LoaderCircleIcon, PlayIcon } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type RefObject } from "react";
import { useShallow } from "zustand/react/shallow";
import type { Actions } from "./actions";
import { answerPlace, type AnswerPlace, type FoundAnswerPath } from "./answerPaths";
import { hostBridge } from "./host";
import { keepScroll, keptScroll, placeAt, scrollPlace, stillAt } from "./conversationScroll";
import { deskOn, deskPhase, holdsFoot, laneCards, livePane, recordCard, sessionFacts, type DeskItem, type DeskRecord, type DeskJob, type DeskOutput, type DeskQueuedMessage, type DeskSessionRef } from "./desk";
import { deskOpener, deskShown, followFrom, followMoved, newSessionBeside, openSession, outputNamed, rightSide, showDesk, type NewSessionFollow } from "./deskRoom";
import { deskViewOf, DESK_SCREEN } from "./deskDesk";
import { CardPlaceContext, type CardPlace } from "./deskFront";
import { DeskViewAreas } from "./ViewAreas";
import { ExplorerTree, type ExplorerScope } from "./ExplorerTree";
import { ThreadPanel } from "./components/desk/thread-panel";
import { SessionChipProvider, type SessionChipSource } from "./components/desk/session-chip";
import { SessionAskCard, SessionHead, SessionRecordFoot, SessionSleepingFoot } from "./components/desk/desk-session";
import { LineButton } from "./LinkSessions";
import { resumeBlock, resumeCheckout } from "./linkPanel";
import { resumeFailureText, resumeReason, resumeRequest, useSessionResume } from "./useSessionResume";
import { OutputShelf } from "./components/desk/output-shelf";
import { GlanceCrumb, GlancePanel } from "./components/desk/glance-panel";
import { rowPulls } from "./prMark";
import { useElapsed } from "./components/elapsed";
import { askIdentity, inputFailure, noticeText } from "./mobile/protocol";
import { allAgents, agentsByPane } from "./paneAgents";
import { commandLabel } from "./shortcutLabels";
import { leaveDesk } from "./keyboard";
import { keepSent, outgoing, pickCommand, pickMention, useComposerTray } from "./deskComposer";
import { refusalText, stageDeskFiles } from "./attachments";
import { DeskChat, DeskEmpty, type DeskChatActions, type DeskNoticeAction } from "./components/desk/desk-chat";
import { DeskAgentBar, DeskComposer, DeskPermissionMenu } from "./components/desk/desk-composer";
import { Separator } from "./components/ui/separator";
import type { MentionInputHandle } from "./components/desk/mention-input";
import { DeskPaneReturn } from "./components/desk/desk-entry";
import { DeskChatColumn, DeskChatHead, DeskRoom } from "./components/desk/desk-room";
import { useComposerMenus } from "./components/desk/composer-menus";
import { DeskSettingsTab } from "./components/desk/desk-settings";
import { useEscapeLayer } from "./components/ui/layer";
import { useInterfaceTranslation } from "./i18n/client";
import { machineName } from "./screenMachine";
import { catalogOf, catalogWorkspaces, checkoutOfPane, localDeviceId, paneRowOf, type AgentRow, type SleepingSession } from "./snapshot";
import { askingSession } from "./sessionWork";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { AgentMark } from "./AgentMark";
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

/**
 * Opens beside Desk the first session a turn starts, as `newSessionBeside`
 * rules (frame J), from the lines and cards this visit of the room sees
 * arrive; the lines there when the room was entered are old.
 */
function useNewSessionBeside(actions: Actions, generation: number | undefined, items: readonly DeskItem[] | undefined, cards: Readonly<Record<string, DeskJob>>) {
  const follow = useRef<NewSessionFollow | null>(null);
  useEffect(() => {
    if (generation === undefined || !items) return;
    if (!follow.current) {
      follow.current = followFrom(generation, items);
      return;
    }
    const next = newSessionBeside(follow.current, generation, items, cards, useUiStore.getState().deskRoom);
    follow.current = next.follow;
    if (next.open) actions.openDeskSession(next.open);
  }, [actions, generation, items, cards]);
}

/**
 * Moves the room to the pane a gone session runs in again (`followMoved`),
 * before the screen paints, so the session panel and each of its actions
 * (files, terminal, input) reach the live session, never the gone pane.
 */
function useFollowMoved() {
  const moved = useShellStore((s) => s.deskJobs?.moved);
  useLayoutEffect(() => {
    if (moved?.length) useUiStore.getState().setDeskRoom((room) => followMoved(room, moved));
  }, [moved]);
}

/** Where the `@` menu stands: the `@` word Esc closed it on, the project ⇥ entered, and the parents the operator folded open or shut. */

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
  const commandSection = useShellStore((s) => s.deskCommands);
  const draft = useUiStore((s) => s.deskDraft);
  const setDraft = useUiStore((s) => s.setDeskDraft);
  const scroller = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const [caret, setCaret] = useState(draft.text.length);
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
  const liveAnswer = useMemo(() => (liveText === null ? null : { text: liveText, mentions: live?.mentions ?? [], missions: live?.missions ?? [] }), [liveText, live?.mentions, live?.missions]);
  // The working line stands from the send to the turn's end, and goes while an approval card waits on the operator (feedback 1 c3).
  const turnLine = useMemo(() => (answering && desk?.approvals_waiting === 0 ? { phase: deskPhase(live?.step, liveText), since: turn } : null), [answering, desk?.approvals_waiting, live?.step, liveText, turn]);
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

  // The conversation keeps its foot in view while the operator reads there (`stick`), and stays where they
  // scrolled otherwise. It follows sizes, not events: a reply's text fills its editor after this pass, a card
  // grows, the box under it grows a line, and Desk opened again would otherwise show the conversation's top.
  // Left above its foot for another screen, it comes back to the item the operator was reading (`keptScroll`),
  // held there while the items around it are drawn again, until the operator scrolls.
  const empty = shown.length === 0 && !answering && queued.length === 0;
  const drawn = desk !== null;
  const scrollKey = `desk:${desk?.conversation.generation ?? 0}`;
  // The conversation the effect last followed: one that changed while Desk stayed open (a new conversation) never
  // comes back, so its place is let go, never read from the new conversation's lines.
  const followed = useRef<string | null>(null);
  useLayoutEffect(() => {
    const box = scroller.current;
    if (!box) return;
    if (followed.current !== null && followed.current !== scrollKey) keepScroll(followed.current, null);
    followed.current = scrollKey;
    let anchor = keptScroll(scrollKey);
    stick.current = anchor === null;
    const pin = () => {
      if (stick.current) box.scrollTop = box.scrollHeight;
      else if (anchor) placeAt(box, anchor);
    };
    let last = box.scrollTop;
    const follow = () => {
      if (anchor && stillAt(box, anchor)) {
        last = box.scrollTop;
        return;
      }
      anchor = null;
      stick.current = holdsFoot(stick.current, last, box.scrollTop, box.scrollHeight - box.scrollTop - box.clientHeight, STICK_PX);
      last = box.scrollTop;
    };
    pin();
    box.addEventListener("scroll", follow, { passive: true });
    const resized = new ResizeObserver(pin);
    resized.observe(box);
    for (const child of box.children) resized.observe(child);
    return () => {
      keepScroll(scrollKey, stick.current ? null : (anchor ?? scrollPlace(box)));
      box.removeEventListener("scroll", follow);
      resized.disconnect();
    };
  }, [empty, drawn, scrollKey]);

  useReadShownCards(scroller, jobs, actions.deskJobRead);
  useNewSessionBeside(actions, desk?.conversation.generation, items, byId);
  useFollowMoved();

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

  // The `/` and `@` menus, the same as a session's box has (`useComposerMenus`); a pick here is a chip in the draft.
  const writeDraft = (next: { draft: typeof draft; caret: number }) => {
    setDraft(next.draft);
    setCaret(next.caret);
    box.current?.place(next.caret);
    box.current?.focus();
  };
  const menus = useComposerMenus({
    text: draft.text,
    caret,
    commands: agent ? { agent: agent.label, list: commandSection?.lists[agent.id] ?? null, unread: commandSection?.unread[agent.id] ?? null, opening: desk?.host.state === "starting" || desk?.host.state === "thinking" } : null,
    onCommandsOpen: agent ? () => actions.deskCommandsCheck(agent.id) : undefined,
    mentions,
    projects: true,
    onCommand: (at, command) => writeDraft(pickCommand(draft, at, command)),
    onMention: (at, mention) => writeDraft(pickMention(draft, at, mention)),
  });

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
    onApproval: (id, option, text) => actions.deskApproval(id, option, text),
    onQueuedRemove: (id) => actions.deskQueuedRemove(id),
    onNoticeAction,
    onCopy: (text, what) => actions.copyText(text, what),
    onMention: (mention) => {
      if (mention.kind === "session") return room.session({ name: mention.name, mark: mention.mark ?? null, agent_kind: mention.agent_kind ?? null, pane_id: mention.id });
      const rest = useShellStore.getState().rest;
      const project = catalogWorkspaces(rest).find((row) => row.checkouts.some((checkout) => checkout.path === mention.id));
      const checkout = project?.checkouts.find((row) => row.path === mention.id);
      if (project && checkout) actions.openWorkspace(project.device_id, project.id, checkout.id);
    },
    onHanded: (paneId) => room.session({ name: "", mark: null, agent_kind: null, pane_id: paneId }),
    onFile: (file) => openAnswerFile(actions, file),
  }), [actions, onNoticeAction, room]);
  const place = useUiStore((s) => s.deskRoom);
  // 한눈에 folded with no session beside Desk: Desk's head carries the way to unfold it.
  const glanceFolded = useShellStore((s) => s.desk?.lane.glance_folded ?? false) && place.session === null;
  useDeskAgentOpens();

  if (!desk) return null;
  const working = jobs.filter((job) => job.state === "working").length;
  // The card or chip that opened what shows is marked (B16).
  const shownJob = place.session ? (jobs.find((job) => job.pane_id === place.session)?.id ?? null) : null;
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
      focused={menus.menuOpen}
      menuOpen={menus.menuOpen}
      menuEmpty={menus.menuEmpty}
      menu={menus.menu}
      onMenuOpenChange={menus.onMenuOpenChange}
      onChange={(text, next) => {
        setDraft({ ...draft, text });
        setCaret(next);
      }}
      onCaret={setCaret}
      onKeyDown={menus.onKeyDown}
      onSend={() => send()}
      onStop={() => actions.deskStop()}
    >
      <DeskPermissionMenu permission={desk.settings.permission} agent={desk.agents.find((agent) => agent.id === desk.selection.agent)?.label ?? null} onPermission={(permission) => actions.deskPermission(permission)} />
      <Separator orientation="vertical" className="mx-xxs data-[orientation=vertical]:h-(--size-icon)" />
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
            <DeskChatColumn head={<DeskChatHead onGlance={glanceFolded ? () => actions.deskGlanceFold(false) : null} />} composer={composer} tray={tray} scrollRef={scroller}>
              {shown.length === 0 && !answering && queued.length === 0 ? (
                <DeskEmpty working={working} asking={jobs.filter((job) => job.state === "asking").length} onExample={(text) => (canSend ? send({ text, mentions: [] }) : setDraft({ text, mentions: [] }))} />
              ) : (
                <DeskChat items={shown} jobs={byId} agents={desk.agents} queued={queued} held={queuedHeld} live={liveAnswer} working={turnLine} selectedJob={shownJob} {...chatActions} />
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
 * pointer; a press opens the session beside Desk, and the chip of the session
 * open there is ringed (frame J).
 */
function useSessionChipSource(room: RoomActions): SessionChipSource {
  const rows = useShellStore((s) => allAgents(s.agents, s.rest?.status?.remote));
  const section = useShellStore((s) => s.deskJobs);
  const open = useUiStore((s) => s.deskRoom.session);
  return useMemo(() => {
    const byPane = new Map(rows.map((row) => [row.pane_id, row]));
    const cards = laneCards(section);
    return { facts: (session) => sessionFacts(byPane, cards, session, section), onOpen: room.session, open };
  }, [rows, section, room, open]);
}

/** Every card Desk has: its jobs, their missions' workers, and the other sessions its conversation names. */
function everyCard(): DeskJob[] {
  return laneCards(useShellStore.getState().deskJobs);
}

/** The output a session's own line names, as `outputNamed` resolves it, on this machine's node. */
function outputOf(job: DeskJob | undefined, output: DeskOutput, place?: { device: string; checkout: string; folder: string } | null, viewId: string | null = null): DeskOutput {
  return outputNamed(job?.outputs ?? [], output, place ?? null, localDeviceId(useShellStore.getState().rest), viewId);
}

/** What a press in Desk opens (PRD desk-2-ui B2, B13, B16, B20, B47). */
type RoomActions = {
  /** A card: its session beside Desk; `답하기` gives its input box the keyboard. */
  job: (job: DeskJob, answer?: boolean) => void;
  /** An output: its session beside Desk, the output in front of Desk's desk. */
  output: (output: DeskOutput) => void;
  /** A mission chip: its map in front of Desk's desk. */
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
        actions.deskOpen({ kind: "output", output: outputOf(job, output), project: job?.project.workspace_id ?? null });
      },
      mission: (missionId) => {
        actions.deskOpen({ kind: "card", card: { kind: "mission", mission: missionId } });
      },
      // D-09, B9: a session is its sidebar row's; by its pane, else by the work name the row shows. A pane that is gone opens
      // the pane its session runs in now, else its record (frame K); a chip with no record cannot be pressed.
      session: (session) => {
        const state = useShellStore.getState();
        const pane = session.pane_id ? livePane(state.deskJobs?.moved, session.pane_id) : allAgents(state.agents, state.rest?.status?.remote).find((row) => row.identity_label === session.name)?.pane_id;
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
 * Desk differ: a mission, a PR or a file opens on Desk's own desk, a session
 * beside Desk.
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

/** A pull request's card on Desk's desk, in the project that lists it (the same PR is one tab, B14). */
function openPullRequest(actions: Actions, url: string, label: string) {
  const job = everyCard().find((one) => one.outputs.some((output) => output.kind === "pr" && output.url === url));
  const listed = job?.outputs.find((output) => output.kind === "pr" && output.url === url);
  actions.deskOpen({ kind: "output", output: listed ?? { id: `pr:${url}`, kind: "pr", label, thumb: null, seen: true, pr_state: null, device_id: null, checkout: null, path: null, url }, project: job?.project.workspace_id ?? null });
}

/**
 * A file chip an answer drew, pressed (feedback 2 frame M [2]): a project's
 * file opens on Desk's desk; a path outside every
 * project never reaches the core, and this computer's file manager only
 * shows it, selected in its folder.
 */
function openAnswerFile(actions: Actions, file: FoundAnswerPath) {
  if (file.kind === "project") return actions.deskOpen({ kind: "file", device_id: file.device, root: file.root, path: file.path });
  const bridge = hostBridge();
  if (!bridge) return useShellStore.getState().noteDiagnostic("desk file chip: no desktop host to reveal an outside path");
  bridge.revealPath(file.path);
}

/** A file Desk's Explorer opened: on Desk's desk, under the session's checkout it lists (B37, B38). */
function openExplorerFile(actions: Actions, scope: ExplorerScope | null, path: string, options: { preview?: boolean; beside?: boolean }) {
  if (!scope) return useShellStore.getState().noteDiagnostic("desk explorer: no session checkout");
  actions.deskOpen({ kind: "file", device_id: scope.device, root: scope.checkout.path, path }, options);
}


/**
 * Desk's room with the session beside it and Desk's own desk (PRD desk-2-ui
 * B1-B26; PRD desk-own-desk B9, B13, B16, B18, B40): Escape hides the desk
 * first, then closes the session panel, each through the shell's Escape
 * owner; hiding the desk gives the keyboard back to what opened it. The desk
 * shows while the room says so and it holds a tab or the Explorer's column.
 */
function DeskRoomView({ actions, room, chat }: { actions: Actions; room: RoomActions; chat: React.ReactNode }) {
  const place = useUiStore((s) => s.deskRoom);
  const lane = useShellStore((s) => s.desk?.lane ?? NO_WIDTHS);
  const view = useShellStore((s) => deskViewOf(s.rest));
  // The session's checkout, which Desk's Explorer lists whatever is in front (B37).
  const scopeCheckout = useShellStore((s) => (place.session ? (checkoutOfPane(s.rest, place.session)?.checkout ?? null) : null));
  const scopeDevice = useShellStore((s) => (place.session ? (checkoutOfPane(s.rest, place.session)?.device ?? null) : null));
  const scope = useMemo<ExplorerScope | null>(() => (scopeCheckout && scopeDevice ? { device: scopeDevice, checkout: scopeCheckout } : null), [scopeCheckout, scopeDevice]);
  const deskActions = useDeskPartActions(actions, room, scope);
  const shown = deskShown(place, view?.layout.display_count ?? 0);
  const openerChord = useShellStore((s) => commandLabel("toggle_lane", s.rest?.ui_state)) || null;
  const side = rightSide(place, lane.glance_folded ?? false);
  // The session beside Desk covers nothing, so it is no Escape layer: Escape closes it from the shell's Escape owner once no
  // layer is open (keyboard.ts). The desk dims the rest and Escape hides it first; it is where the operator works, so it
  // covers nothing for the close chords, and ⌘W closes its tab (B6).
  useEscapeLayer(shown, () => actions.deskHideDesk(), false, true);
  // The keyboard goes back to the card or chip that opened the desk.
  const opener = useRef<HTMLElement | null>(null);
  useLayoutEffect(() => {
    if (shown) {
      if (!opener.current) opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      return;
    }
    const back = opener.current;
    opener.current = null;
    if (back?.isConnected) back.focus();
  }, [shown]);
  return (
    <DeskRoom
      chat={chat}
      widths={lane}
      expanded={place.expanded}
      glance={side === "glance"}
      session={side === "session" && place.session ? <DeskSessionPanel key={place.session} paneId={place.session} actions={actions} deskActions={deskActions} room={room} /> : side === "glance" ? <DeskGlance actions={actions} room={room} /> : null}
      panel={shown ? <DeskDeskView view={view} actions={deskActions} room={room} scope={place.explorer ? scope : null} expanded={place.expanded} /> : null}
      opener={deskOpener(place, view?.layout.display_count ?? 0)}
      openerChord={openerChord}
      onOpener={() => actions.deskToggleDesk()}
      onDim={() => actions.deskHideDesk()}
      onWidth={(panel, width) => actions.deskLaneWidth(panel, width)}
    />
  );
}

/**
 * 한눈에 at Desk's right while no session is open beside it: a session row
 * opens that session beside Desk, a file row opens it in front of Desk's desk
 * and turns it seen; its tab, its groups' folds and `▯|`, which folds it
 * whole, are the core's.
 */
function DeskGlance({ actions, room }: { actions: Actions; room: RoomActions }) {
  const section = useShellStore((s) => s.deskJobs);
  const tab = useShellStore((s) => s.desk?.lane.glance_tab);
  const folds = useShellStore((s) => s.desk?.lane.glance_groups);
  const cards = useMemo(() => laneCards(section), [section]);
  if (!section) return null;
  return (
    <GlancePanel
      glance={section.glance}
      cards={cards}
      tab={tab}
      folds={folds}
      onTab={(next) => actions.deskGlanceTab(next)}
      onGroup={(group, folded) => actions.deskGlanceGroup(group, folded)}
      onSession={(pane) => room.session({ name: "", mark: null, agent_kind: null, pane_id: pane })}
      onFile={(file) => actions.deskOpen({ kind: "output", output: file })}
      onFold={() => actions.deskGlanceFold(true)}
    />
  );
}

/**
 * B35: a file Desk's agent puts on Desk's desk shows it while this window
 * shows Desk; on any other screen nothing moves, and the tab waits on the desk
 * until something next opens there. Only a move of the core's count after the room opened shows it.
 */
function useDeskAgentOpens() {
  const opened = useShellStore((s) => deskViewOf(s.rest)?.opened ?? null);
  const seen = useRef(opened);
  useEffect(() => {
    const before = seen.current;
    seen.current = opened;
    // Only a count that grew is an open: Desk switched on again, or a core
    // that restarted from zero, shows nothing.
    if (opened !== null && before !== null && opened > before) useUiStore.getState().setDeskRoom(showDesk);
  }, [opened]);
}

/**
 * Desk's own desk (PRD desk-own-desk D-01, B1-B6, B12, B36, B37): the
 * Explorer's column of the session's checkout when `파일` opened it, and the
 * View areas a Workspace's Agent tab draws, kept by the core. The keyboard
 * owner names it as Desk's (`DESK_SCREEN`), so ⌘W and the area keys act on
 * it; a worker picked on a mission's map opens beside Desk.
 */
function DeskDeskView({ view, actions, room, scope, expanded }: { view: import("./snapshot").DeskViewSnapshot | null; actions: Actions; room: RoomActions; scope: ExplorerScope | null; expanded: boolean }) {
  const { t } = useInterfaceTranslation();
  const place = useMemo<CardPlace>(() => ({ onMissionNode: (node) => (node.pane_id ? room.session({ name: "", mark: null, agent_kind: null, pane_id: node.pane_id }) : undefined) }), [room]);
  const ends = useMemo(() => ({ onClose: () => actions.deskHideDesk(), expanded, onExpand: () => actions.deskToggleExpanded() }), [actions, expanded]);
  return (
    <div className="flex min-h-0 min-w-0 flex-1" data-workspace-screen={DESK_SCREEN} data-desk-desk={view?.layout.display_count ?? 0}>
      {scope ? <div className="flex w-(--size-tree-ideal) min-w-0 shrink-0 flex-col border-r border-border bg-sidebar" data-desk-tool-column="explorer"><ExplorerTree actions={actions} scope={scope} /></div> : null}
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        {view && view.layout.display_count > 0 ? (
          <CardPlaceContext.Provider value={place}>
            <DeskViewAreas view={view} ends={ends} actions={actions} />
          </CardPlaceContext.Provider>
        ) : <p className="m-auto px-md text-caption text-muted-foreground" data-desk-panel-empty="true">{t("desk.room.explorerEmpty")}</p>}
      </div>
    </div>
  );
}

const NO_QUEUED: readonly DeskQueuedMessage[] = [];
const NO_OUTPUTS: readonly DeskOutput[] = [];
const NO_WIDTHS: import("./snapshot").DeskLane = { view_width: null, session_width: null };

/**
 * The session open beside Desk (PRD desk-2-ui D-01, D-09-D-11, D-20, D-28,
 * B2, B6-B15, B27, B30, B34, B43, B47): its head as the pane head draws it,
 * its conversation read from its pane, the question it waits on as a card,
 * and its input box, or `에이전트 깨우기` in the box's place while it sleeps.
 */
function DeskSessionPanel({ paneId, actions, deskActions, room }: { paneId: string; actions: Actions; deskActions: Actions; room: RoomActions }) {
  const listed = useShellStore((s) => agentsByPane(allAgents(s.agents, s.rest?.status?.remote)).has(paneId));
  const record = useShellStore((s) => (listed ? null : ((s.deskJobs?.records ?? []).find((one) => one.pane_id === paneId) ?? null)));
  return record ? <DeskRecordPanel record={record} actions={actions} deskActions={deskActions} room={room} /> : <DeskLivePanel paneId={paneId} actions={actions} deskActions={deskActions} room={room} />;
}

/** The way back to 한눈에 above the session beside Desk, naming it as its head does: each panel hands it the title it gives its head. */
function SessionCrumb({ title, kind, actions }: { title: string; kind: string | null; actions: Actions }) {
  return <GlanceCrumb title={title} lead={kind ? <AgentMark kind={kind} /> : null} onBack={() => actions.deskGlanceBack()} />;
}

/**
 * A session whose pane is gone, opened beside Desk from Hide's record of it
 * (feedback 2, frame K, 다): its head with `끝남` and no buttons, `‹ 한눈에` above it, its machine
 * and pull requests under it, its conversation read where it ran as any
 * session's, and in the input box's place the session record row's
 * `이어서 하기`, which resumes it as that row does, or the one line the app
 * gives why it cannot.
 */
function DeskRecordPanel({ record, actions, deskActions, room }: { record: DeskRecord; actions: Actions; deskActions: Actions; room: RoomActions }) {
  const { t } = useInterfaceTranslation();
  const job = useMemo(() => recordCard(record), [record]);
  const ownJob = useShellStore((s) => s.deskJobs?.jobs.some((one) => one.pane_id === record.pane_id) ?? false);
  const node = useShellStore((s) => localDeviceId(s.rest));
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const workspaces = useShellStore((s) => s.rest?.navigator?.workspaces);
  const remote = useShellStore((s) => s.rest?.status?.remote);
  const device = record.session.device_id;
  // Only another device is named: this machine goes without saying (Doop H 다).
  const machine = device !== node ? machineName(devices, device, t) : null;
  // Where it resumes and why it cannot, as the session record screen decides it (`SessionRecord.tsx`, `linkPanel.ts`).
  const checkout = resumeCheckout({ device_id: device, cwd: record.cwd }, catalogOf(workspaces, remote), null, node);
  // Where it ran, from its record, since its pane is gone: its machine and its own folder, where its relative paths
  // and the files it opened are read, as its pane's are while it runs.
  const folder = record.cwd ?? record.project;
  const place = useMemo<AnswerPlace>(() => ({ device, cwd: folder }), [device, folder]);
  const filePlace = checkout ? { device, checkout: checkout.path, folder: folder ?? checkout.path } : null;
  const live = useShellStore((s) => s.connection === "live");
  const blocked = resumeBlock({ agent: record.session.agent, file: record.path ? "present" : "missing", device_id: device }, checkout, devices, node);
  const reason = resumeReason(blocked, live, t);
  // The resumed session opens beside Desk, as any session Desk opens does, not in its own tab.
  const { resume, start } = useSessionResume((pane) => actions.openDeskSession(pane));
  const resumeNow = () => {
    const send = reason ? null : resumeRequest(actions.startAgent, { ...record.session, path: record.path }, checkout, node);
    if (send) start(send);
  };
  const resuming = resume.request !== null && !resume.uncertain;
  const failure = resumeFailureText(resume, t);
  const button = (
    <>
      {failure ? <span role="alert" className="mr-xs min-w-0 break-words text-warning" data-desk-session-resume-failed="true">{failure}</span> : null}
      <LineButton icon={resuming ? <LoaderCircleIcon aria-hidden="true" className="animate-spin" /> : <PlayIcon aria-hidden="true" />} label={t(resume.uncertain ? "groups.confirm" : resuming ? "links.resuming" : "links.resume")} blocked={null} busy={resuming} onClick={resumeNow} data="resume" id={record.session.session_id} />
    </>
  );
  return (
    <>
    <SessionCrumb title={record.title} kind={record.agent_kind} actions={actions} />
    <ThreadPanel
      job={job}
      paneId={record.pane_id}
      actions={actions}
      startedByDesk={ownJob}
      head={
        <SessionHead
          paneId={record.pane_id}
          title={record.title}
          job={job}
          row={null}
          pane={null}
          machine={machine}
          disconnected={false}
          sleeping={false}
          ended={{ pulls: record.pull_requests }}
          explorerOpen={false}
          filesChord={null}
          terminalChord={null}
          deskActions={deskActions}
          onFiles={() => undefined}
          onTerminal={() => undefined}
          onOwnTab={() => undefined}
        />
      }
      foot={<SessionRecordFoot endedAt={record.ended_at_unix_ms} resume={button} blocked={reason} />}
      outputOf={(output, viewId) => outputOf(undefined, output, filePlace, viewId)}
      onOutput={(output) => actions.deskOpen({ kind: "output", output, project: job?.project.workspace_id ?? null })}
      onMission={room.mission}
      onSession={room.session}
      onCopy={(text, what) => actions.copyText(text, what)}
      onFile={(file) => openAnswerFile(actions, file)}
      place={place}
    />
    </>
  );
}

/** The session beside Desk while its pane is there, or was, with no record to read in its place. */
function DeskLivePanel({ paneId, actions, deskActions, room }: { paneId: string; actions: Actions; deskActions: Actions; room: RoomActions }) {
  const { t } = useInterfaceTranslation();
  const job = useShellStore((s) => (s.deskJobs ? laneCards(s.deskJobs).find((one) => one.pane_id === paneId) ?? null : null));
  const ownJob = useShellStore((s) => s.deskJobs?.jobs.some((one) => one.pane_id === paneId) ?? false);
  const row = useShellStore((s) => agentsByPane(allAgents(s.agents, s.rest?.status?.remote)).get(paneId) ?? null);
  const pane = useShellStore((s) => paneRowOf(s.rest, paneId));
  const checkoutPath = useShellStore((s) => checkoutOfPane(s.rest, paneId)?.checkout.path ?? null);
  // The session's own folder and machine, where its relative paths are read.
  const place = useShellStore(useShallow((s) => answerPlace(s.rest, paneId)));
  // Only another device is named: this machine goes without saying (Doop H 다).
  const machine = useShellStore((s) => (row && row.device_id !== localDeviceId(s.rest) ? machineName(s.rest?.navigator?.devices, row.device_id, t) : null));
  const pulls = useMemo(() => (row ? rowPulls(row) : []), [row]);
  const explorer = useUiStore((s) => s.deskRoom.explorer);
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
  // Reading the session's card reads its row, as reading it in the chat does.
  const unread = ownJob && job && !job.read ? job.id : null;
  useEffect(() => {
    if (unread) actions.deskJobRead(unread);
  }, [actions, unread]);
  const write = () => useUiStore.getState().setDeskRoom((now) => openSession(now, paneId, { answer: true }));
  return (
    <>
    <SessionCrumb title={title} kind={row?.agent_kind ?? job?.agent_kind ?? null} actions={actions} />
    <ThreadPanel
      job={job}
      paneId={paneId}
      actions={actions}
      startedByDesk={ownJob}
      answer={answer}
      head={
        <>
        <SessionHead
          paneId={paneId}
          title={title}
          job={job}
          row={row}
          pane={pane}
          machine={machine}
          disconnected={job?.disconnected ?? false}
          sleeping={sleeping}
          explorerOpen={explorer}
          filesChord={filesChord}
          terminalChord={terminalChord}
          deskActions={deskActions}
          onFiles={() => actions.deskToggleExplorer()}
          onTerminal={() => actions.deskTerminal()}
          onOwnTab={() => actions.openListedAgent(paneId)}
        />
        <OutputShelf
          outputs={job?.outputs ?? NO_OUTPUTS}
          more={job?.outputs_more ?? false}
          pulls={pulls}
          onOpen={(output) => actions.deskOpen({ kind: "output", output, project: job?.project.workspace_id ?? null })}
          onPull={(pull, external) => (external ? actions.openLink(pull.url, true) : deskActions.openSessionPullRequest({ workspace_id: null, url: pull.url, number: pull.number, reference: pull.reference }, { pane: paneId }))}
          row={row}
        />
        </>
      }
      ask={job?.state === "asking" ? <DeskSessionAsk paneId={paneId} deviceId={row?.device_id ?? null} job={job} row={row} actions={actions} onWrite={write} /> : null}
      asks={row?.open_asks?.items}
      onAsk={(id, status) => {
        const asking = row ? askingSession(row) : null;
        if (asking) actions.setMissedStatus(asking, id, status);
      }}
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
      outputOf={(output, viewId) => outputOf(job ?? undefined, output, row && checkoutPath ? { device: row.device_id, checkout: checkoutPath, folder: place?.cwd ?? checkoutPath } : null, viewId)}
      onOutput={(output) => actions.deskOpen({ kind: "output", output })}
      onMission={room.mission}
      onSession={room.session}
      onCopy={(text, what) => actions.copyText(text, what)}
      onFile={(file) => openAnswerFile(actions, file)}
      place={place}
    />
    </>
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
  if (!ask || !deviceId) return null;
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
