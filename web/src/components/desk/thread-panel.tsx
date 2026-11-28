import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { agentAdapter } from "../../agentAdapters";
import { catalogFor, type ProviderKind } from "../../agentPicker";
import type { Actions } from "../../actions";
import { refusalText, submitThreadInput } from "../../attachments";
import type { DeskJob, DeskMention } from "../../desk";
import { useInterfaceTranslation } from "../../i18n/client";
import type { MessageKey } from "../../i18n/catalogs";
import { allAgents } from "../../paneAgents";
import { fileWords, keepSent, mentionAt, pickCommand, pickWords, useComposerTray } from "../../deskComposer";
import { sessionBarSlots, sessionMentions, sessionWords, switchCommand, useMentionFiles } from "../../sessionInput";
import { checkoutOfPane, type DeskAgentId, type SleepingSession } from "../../snapshot";
import { conversationSleep } from "../../sleep";
import { SleepCard } from "../sleep-card";
import { useSessionThread } from "../../sessionThread";
import { sessionDraft, visitSessionTerminal } from "../../sessionDrafts";
import { remoteTargetOfPane } from "../../remote";
import "../../sessionDraftSends";
import { useShellStore } from "../../store";
import { useUiStore } from "../../ui";
import { threadInputHeld, threadScreen } from "../../threadView";
import type { ReactNode } from "react";
import type { AskLine } from "../open-asks";
import { useComposerMenus } from "./composer-menus";
import { SessionAgentBar } from "./desk-composer";
import { FileDropZone } from "./file-drop";
import { QueueGroup } from "./queue-group";
import { SessionThreadAsk } from "./session-ask";
import { NO_MENTIONS, ThreadView, type ThreadPartActions } from "./thread-view";

import { Button } from "../ui/button";
import { Hint } from "../ui/tooltip";

const NO_CHOICE: { model?: string; effort?: string } = {};

/** The refusals a thread's input words itself; any other is an attachment's, worded as a terminal attachment's is. */
const INPUT_WORDS = new Set(["pane_changed", "pane_gone", "closed", "asleep", "uncertain", "unavailable", "device_unreachable", "too_long", "control_characters", "attachment_failed", "input_refused", "switch_unsupported", "needs_model", "invalid_model", "invalid_effort", "invalid_request", "agent_asking", "agent_busy", "draft", "agent_not_working", "interrupt_unsupported"]);

/**
 * A session's conversation on screen, connected to the core (PRD desk-2-ui
 * D-01, D-20, B2, B15, B30): it opens the session thread of the pane while
 * it is shown and closes it after, draws the record as the core reads it
 * on, reads the page before when the conversation is scrolled to its top,
 * and types the input box's words and files into that pane. The box has
 * Desk's `/` and `@` menus and, under it, the session's model and effort
 * with its agent fixed (mission-106). While the session sleeps the
 * conversation stays readable, dimmed, with the sleep card in its middle
 * and no box (`conversationSleep`, `SleepCard`). Its head (with the outputs
 * shelf under it), the agent's choice card, the questions Hide found and
 * what stands in an ended session's box's place are the caller's.
 */
export function ThreadPanel({ paneConversation = false, job, paneId = job?.pane_id ?? null, actions, mentions = NO_MENTIONS, startedByDesk = false, focused = false, answer = false, onAnswered, head = null, asks, onAsk, foot = null, asleep = null, onSessionWake, onTerminal, ...parts }: {
  /** The session's card; a session that is no job of Desk's opens before its card arrives, by its pane (D-87). */
  job: DeskJob | null;
  paneConversation?: boolean;
  /** The pane whose thread this is; its card's when not given. */
  paneId?: string | null;
  actions: Pick<Actions, "openThread" | "closeThread" | "readOlderThread" | "switchThread" | "observeCatalog" | "requestFileIndex" | "deskCommandsCheck" | "inputQueue" | "interruptThread" | "wakeAgent" | "wakeSleepingSession" | "askPane" | "answerPane" | "deskTerminal">;
  mentions?: readonly DeskMention[];
  startedByDesk?: boolean;
  focused?: boolean;
  /** A press asks for this session's input box and no box has taken it yet (`답하기`, B29, or opening the conversation over its pane): the box takes the keyboard. */
  answer?: boolean;
  /** The box took the keyboard for that press, which is spent. */
  onAnswered?: () => void;
  /** The panel's head, drawn once the card is here. */
  head?: ReactNode;
  asks?: readonly AskLine[];
  /** 무시하기 and 되돌리기 on one of `asks`, for the session they wait in; without it no question line is drawn. */
  onAsk?: (id: string, status: "open" | "dismissed") => void;
  foot?: ReactNode;
  /** The sleeping session this one is while its sleep closed its pane (Pi, OpenCode, omp), which wakes in a new pane; read only while the pane has no agent row. */
  asleep?: SleepingSession | null;
  /** Its wake was asked for, so the caller can follow the new pane. */
  onSessionWake?: () => void;
  /** The session's terminal; none for a session whose pane is gone. */
  onTerminal?: () => void;
} & ThreadPartActions) {
  const { t } = useInterfaceTranslation();
  const { thread, history, taken, reclaim } = useSessionThread(actions, paneId, paneConversation);
  const toTerminal = onTerminal ? () => {
    if (paneId) useUiStore.setState((s) => ({ sessionDrafts: visitSessionTerminal(s.sessionDrafts, paneId) }));
    onTerminal();
  } : undefined;
  const plans = useShellStore((s) => s.rest?.plans);
  const missions = useShellStore((s) => s.rest?.missions);
  const rows = useShellStore((s) => allAgents(s.agents, s.rest?.status?.remote));
  const screen = useMemo(() => threadScreen(thread, history, { plans, missions }, rows), [thread, history, plans, missions, rows]);

  // The words are the pane's draft, kept while another session is open or Desk is left, and cleared once sent.
  // An edit that would make the words longer past one send is not taken, and the line says so; the words stay.
  const text = useUiStore((s) => (paneId ? sessionDraft(s.sessionDrafts, paneId) : ""));
  const setText = (value: string) => {
    if (!paneId) return;
    const device = remoteTargetOfPane(useShellStore.getState().rest, paneId);
    if (!useUiStore.getState().setSessionDraft(paneId, device, value)) setNotice(words("too_long"));
  };
  const [caret, setCaret] = useState(0);
  const box = useRef<HTMLTextAreaElement>(null);
  const tray = useComposerTray();
  // The request a send is waiting on and the tiles it carried, while this panel stands; its words wait beside the
  // pane's draft (`sessionDraftSends.ts`), so a send still unanswered when the panel closed holds the next one too.
  const [sent, setSent] = useState<{ request: string; tiles: string[] } | null>(null);
  const pending = useRef<typeof sent>(null);
  const holdSent = (next: typeof sent) => { pending.current = next; setSent(next); };
  const waiting = useUiStore((s) => (paneId ? s.sessionDrafts.some((draft) => draft.pane === paneId && draft.sending) : false));
  const uncertain = useUiStore((s) => (paneId ? s.sessionDrafts.some((draft) => draft.pane === paneId && draft.uncertain) : false));
  const [notice, setNotice] = useState<string | null>(null);
  const words = (reason: string) => (INPUT_WORDS.has(reason) ? t(`desk.thread.input.${reason}` as MessageKey) : refusalText(reason, t));

  // A send takes the words out of the box at once, and `sessionDraftSends.ts` gives them back on a refusal or lets
  // them go once the pane took them, whether this panel still stands or not; here only the line and the files the
  // send carried follow its direct completion. Subscribe to every frame: React may batch two answers into one
  // render, and another request's last result must never hide this request's result.
  useEffect(() => useShellStore.subscribe((state, previous) => {
    const input = state.threadInputResult;
    const refusal = state.composerRefusal;
    const current = pending.current;
    if (!current) return;
    if (input && input !== previous.threadInputResult && input.request_id === current.request && input.state !== "sending") {
      pending.current = null;
      setSent(null);
      if (input.state === "refused") setNotice(input.reason ? words(input.reason) : null);
      else {
        current.tiles.forEach(tray.remove);
        setNotice(null);
      }
    } else if (refusal && refusal !== previous.composerRefusal && refusal.request_id === current.request) {
      pending.current = null;
      setSent(null);
      setNotice(words(refusal.reason));
    }
  }), [tray.remove, t]);
  useEffect(() => {
    if (uncertain) { holdSent(null); setNotice(words("uncertain")); }
  }, [uncertain]);

  const held = taken ? "closed" : threadInputHeld(thread);
  const canSend = () => {
    if (useShellStore.getState().connection === "live") return true;
    setNotice(words("forward_failed"));
    return false;
  };
  const send = () => {
    const files = tray.outgoing();
    if (!thread?.pane_id || !paneId || held !== null || sent || waiting || uncertain || !files) return;
    if (!canSend()) return;
    const request = submitThreadInput(thread.generation, thread.pane_id, text, files.map(({ name, bytes }) => ({ name, bytes })));
    if (request && typeof request === "object") {
      setNotice(words(request.refused));
    } else if (request) {
      // The bubble the record will show names these files by their stages, here and on a device.
      files.forEach((file, index) => keepSent(`${request}-${index}`, file.name, file.image));
      holdSent({ request, tiles: files.map((file) => file.id) });
      useUiStore.getState().sendSessionDraft(paneId, remoteTargetOfPane(useShellStore.getState().rest, paneId), request, text);
      setNotice(null);
    } else {
      setNotice(words("forward_failed"));
    }
  };
  // The `/` and `@` menus Desk's composer has: the session's agent's own commands, and the session's project's
  // files above Desk's sessions. A pick is written as the agent reads it in its terminal.
  const row = rows.find((one) => one.pane_id === paneId);
  // Asleep: the conversation stays to read, dimmed, and the card in its middle wakes it; nothing is typed into it.
  const sleep = paneId ? conversationSleep(paneId, { row: row ?? null, job, session: asleep }, actions, onSessionWake) : null;
  // What the agent does is its sidebar row's, the value its mark and its terminal's status show, beside Desk and over
  // its pane alike, whatever its card says of its place (moved out of Desk's group, or no card of Desk's at all).
  const asking = row?.state.asking === true;
  const working = row?.state.working === true && !asking;
  // The agent's own question, read from its screen and answered in place, stands while its row asks.
  const ask = asking && row && paneId && !sleep && !foot ? (
    <SessionThreadAsk paneId={paneId} deviceId={row.device_id} job={job} row={row} actions={actions} onTerminal={toTerminal} onWrite={() => box.current?.focus()} />
  ) : null;
  const kind = row?.agent_kind ?? null;
  const adapter = kind ? agentAdapter(kind) : undefined;
  const commandSection = useShellStore((s) => s.deskCommands);
  const deskMentions = useShellStore((s) => s.deskMentions);
  const rest = useShellStore((s) => s.rest);
  const named = useMemo(() => sessionMentions(deskMentions, rest, paneId), [deskMentions, rest, paneId]);
  const checkout = paneId ? checkoutOfPane(rest, paneId) : null;
  const place = useMemo(() => (checkout ? { device: checkout.device, root: checkout.checkout.path } : null), [checkout?.device, checkout?.checkout.path]);
  const at = mentionAt(text, caret);
  const files = useMentionFiles(actions.requestFileIndex, place, at?.query ?? null);
  // A pick's words land on the next render, and the caret is placed after them in that render's layout pass.
  const placed = useRef<number | null>(null);
  useLayoutEffect(() => {
    const at = placed.current;
    if (at === null) return;
    placed.current = null;
    box.current?.focus();
    box.current?.setSelectionRange(at, at);
  }, [text]);
  const write = (next: { text: string; caret: number }) => {
    placed.current = next.caret;
    setText(next.text);
    setCaret(next.caret);
  };
  const menus = useComposerMenus({
    text,
    caret,
    commands: adapter ? { agent: adapter.picker_label, list: commandSection?.lists[adapter.agent_cli as DeskAgentId] ?? null, unread: commandSection?.unread[adapter.agent_cli as DeskAgentId] ?? null } : null,
    // Only an agent Desk reads lists for: one the section names, with its list or why it has none.
    onCommandsOpen: adapter && commandSection && (adapter.agent_cli in commandSection.lists || adapter.agent_cli in commandSection.unread) ? () => actions.deskCommandsCheck(adapter.agent_cli as DeskAgentId) : undefined,
    mentions: named,
    projects: false,
    files,
    onCommand: (slash, command) => {
      const next = pickCommand({ text, mentions: [] }, slash, command);
      write({ text: next.draft.text, caret: next.caret });
    },
    onMention: (place, mention) => write(pickWords(text, place, sessionWords(mention))),
    onFile: (place, file) => write(pickWords(text, place, fileWords(file))),
  });

  // The model and effort under the box, the agent fixed: a change types the agent's own line into the session.
  const ai = useShellStore((s) => s.rest?.status?.background_ai);
  const startable = adapter?.can_start === true;
  useEffect(() => (startable ? actions.observeCatalog() : undefined), [actions, startable]);
  const catalog = adapter && startable ? catalogFor(ai, adapter.agent_cli as ProviderKind) : null;
  const choice = thread?.choice ?? NO_CHOICE;
  const slots = adapter ? sessionBarSlots(adapter.agent_cli, choice, catalog, t) : null;
  const change = (to: { model: string } | { effort: string }) => {
    if (!thread?.pane_id || !paneId || held !== null || sent || waiting || uncertain || ask !== null) return;
    if (!canSend()) return;
    const request = actions.switchThread(thread.generation, thread.pane_id, to);
    if (request) {
      holdSent({ request, tiles: [] });
      useUiStore.getState().sendSessionDraft(paneId, remoteTargetOfPane(useShellStore.getState().rest, paneId), request, "", true);
      setNotice(null);
    } else {
      setNotice(words("forward_failed"));
    }
  };
  // While the agent works, ■ types the keys its `[interrupt]` names and pauses its waiting line; ↵ keeps adding to the
  // line (PRD desk-queue B10-B12). A line's `보내기` waits until it rests and asks nothing (B16).
  const stop = () => {
    if (!thread?.pane_id || !paneId || held !== null || sent || waiting || uncertain) return;
    if (!canSend()) return;
    const request = actions.interruptThread(thread.generation, thread.pane_id);
    if (request) {
      holdSent({ request, tiles: [] });
      useUiStore.getState().sendSessionDraft(paneId, remoteTargetOfPane(useShellStore.getState().rest, paneId), request, "", true);
      setNotice(null);
    } else {
      setNotice(words("forward_failed"));
    }
  };
  const queue = thread?.queue && paneId ? (
    <QueueGroup
      state={thread.queue.state}
      lines={thread.queue.lines}
      cleared={thread.queue.cleared}
      stalled={thread.queue.stalled === true}
      sendable={row !== undefined && !working && !asking}
      dimmed={!thread.connected}
      onAction={(action, id) => actions.inputQueue(paneId, action, id)}
    />
  ) : null;
  const bar = adapter && slots && !foot && !sleep ? (
    <SessionAgentBar
      agent={{ kind: adapter.agent_cli, label: adapter.picker_label }}
      model={slots.model}
      effort={slots.effort}
      switchHint={{ model: switchCommand(adapter.switch?.model), effort: switchCommand(adapter.switch?.effort) }}
      // A question card on screen is answered first: the line's Enter would answer it.
      held={held !== null || sent !== null || uncertain || ask !== null}
      onModel={(model) => change({ model })}
      onEffort={(effort) => change({ effort })}
    />
  ) : null;

  // One ask per state the core shows: scroll events at the top come faster
  // than its answer, and a read that failed is asked again only after the
  // core moved.
  const askedOlder = useRef(false);
  useEffect(() => {
    askedOlder.current = false;
  }, [thread?.generation, thread?.older]);
  const reachTop = () => {
    if (taken || askedOlder.current || !thread?.has_older || (thread.older !== "idle" && thread.older !== "failed")) return;
    askedOlder.current = true;
    actions.readOlderThread(thread.generation);
  };

  // A `답하기` press names the session it was pressed for, and that session's box takes the keyboard once it stands,
  // whether the session was open at the press or opened because of it (its card and record may still be on their way).
  const root = useRef<HTMLDivElement>(null);
  const boxStands = paneId !== null && !foot && !sleep && !taken;
  useEffect(() => {
    if (!answer || !boxStands) return;
    const input = root.current?.querySelector<HTMLTextAreaElement>("[data-desk-composer-input]");
    if (!input) return;
    input.focus();
    onAnswered?.();
  }, [answer, boxStands]);

  if (!paneId && !foot) {
    // The thread opened from the first frame and its card is on its way, or the session has no card any more (its
    // pane closed): the head and the way back above it stand either way (B6).
    return (
      <div ref={root} className="flex min-h-0 min-w-0 flex-1 flex-col" data-desk-thread-panel={paneId ?? ""} data-desk-thread-pending="true">
        {head}
        <p role="status" className="p-md text-caption text-muted-foreground">{t("desk.thread.reading")}</p>
      </div>
    );
  }
  return (
    // The whole panel, its head, conversation and box, takes a dropped file into the box while the box stands (Doop L 나).
    <FileDropZone ref={root} tray={foot || sleep || taken ? null : tray} className="relative flex min-h-0 min-w-0 flex-1 flex-col" data-desk-thread-panel={paneId ?? ""}>
      {head}
      <div className="relative flex min-h-0 min-w-0 flex-1 flex-col">
      <div inert={taken} className={`flex min-h-0 min-w-0 flex-1 flex-col ${taken || thread?.connected === false ? "opacity-(--opacity-disabled)" : sleep ? "opacity-(--opacity-dimmed)" : ""}`} data-thread-taken={taken ? "true" : "false"} data-thread-asleep={sleep ? "true" : undefined}>
      <ThreadView
        key={paneId ?? ""}
        scrollKey={paneId ? `thread:${paneId}` : null}
        job={job}
        running={working}
        workingSince={rows.find((row) => row.pane_id === paneId)?.last_prompt_at_unix_ms ?? null}
        lines={screen.lines}
        reading={thread === null || screen.reading}
        failure={screen.failure}
        mentions={mentions}
        startedByDesk={startedByDesk}
        ask={ask}
        asks={asks}
        onAsk={onAsk}
        foot={foot}
        composer={sleep ? null : {
          value: text,
          tray,
          focused,
          held: held !== null || sent !== null || waiting || uncertain,
          notice: held === "pane_changed" || held === "pane_gone" ? words(held) : notice,
          textRef: box,
          menus,
          bar,
          queue,
          onStop: working && adapter?.interrupt ? stop : undefined,
          onChange: (value, next) => {
            setText(value);
            setCaret(next);
          },
          onCaret: setCaret,
          onSend: send,
        }}
        onTerminal={toTerminal}
        onReachTop={reachTop}
        olderLeft={thread?.has_older ?? false}
        {...parts}
      />
      {uncertain && toTerminal ? <div className="px-md pb-md"><Button variant="text-link" size="inline" onClick={toTerminal}>{t("desk.thread.toTerminal")}</Button></div> : null}
      </div>
      {/* The card stands over the conversation's middle and takes only its own presses: around it the record still
          scrolls, reads the pages before, selects and copies. It stands clear of the window's foot, where the corner's
          notices sit, so it holds no notice floor as the box it replaces did. */}
      {sleep && paneId && !taken ? (
        <div className="pointer-events-none absolute inset-0 flex items-center justify-center p-md" data-thread-sleep-card="true">
          <SleepCard paneId={paneId} view={sleep} className="pointer-events-auto max-w-full rounded-lg bg-popover px-lg py-md text-popover-foreground shadow-overlay" />
        </div>
      ) : null}
      </div>
      {taken ? <div className="absolute inset-0 flex items-center justify-center p-md">
        <Hint label={t("panes.conversation.onePlace")}>
          <Button variant="secondary" size="sm" onClick={reclaim} data-thread-reclaim="true">{t("panes.conversation.reclaim")}</Button>
        </Hint>
      </div> : null}
    </FileDropZone>
  );
}
