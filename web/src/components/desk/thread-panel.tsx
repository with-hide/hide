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
import { checkoutOfPane } from "../../snapshot";
import { useSessionThread } from "../../sessionThread";
import { useShellStore } from "../../store";
import { threadInputHeld, threadScreen } from "../../threadView";
import type { ReactNode } from "react";
import type { AskLine } from "../open-asks";
import { useComposerMenus } from "./composer-menus";
import { SessionAgentBar } from "./desk-composer";
import { FileDropZone } from "./file-drop";
import { NO_MENTIONS, ThreadView, type ThreadPartActions } from "./thread-view";

const NO_CHOICE: { model?: string; effort?: string } = {};

/** The refusals a thread's input words itself; any other is an attachment's, worded as a terminal attachment's is. */
const INPUT_WORDS = new Set(["pane_changed", "pane_gone", "closed", "asleep", "uncertain", "unavailable", "device_unreachable", "too_long", "control_characters", "attachment_failed", "input_refused", "switch_unsupported", "needs_model", "invalid_model", "invalid_effort", "invalid_request"]);

/**
 * A session's conversation on screen, connected to the core (PRD desk-2-ui
 * D-01, D-20, B2, B15, B30): it opens the session thread of the pane while
 * it is shown and closes it after, draws the record as the core reads it
 * on, reads the page before when the conversation is scrolled to its top,
 * and types the input box's words and files into that pane. The box has
 * Desk's `/` and `@` menus and, under it, the session's model and effort
 * with its agent fixed (mission-106). Its head (with
 * the outputs shelf under it), the agent's choice card, the questions Hide
 * found and what stands in the box's place while it sleeps are the caller's.
 */
export function ThreadPanel({ job, paneId = job?.pane_id ?? null, actions, mentions = NO_MENTIONS, startedByDesk = false, focused = false, answer = 0, head = null, ask = null, asks, onAsk, foot = null, onTerminal, ...parts }: {
  /** The session's card; a session that is no job of Desk's opens before its card arrives, by its pane (D-87). */
  job: DeskJob | null;
  /** The pane whose thread this is; its card's when not given. */
  paneId?: string | null;
  actions: Pick<Actions, "openThread" | "closeThread" | "readOlderThread" | "switchThread" | "observeCatalog" | "requestFileIndex">;
  mentions?: readonly DeskMention[];
  startedByDesk?: boolean;
  focused?: boolean;
  /** Moves when `답하기` asks for this thread's input box (B29): the box takes the keyboard. */
  answer?: number;
  /** The panel's head, drawn once the card is here. */
  head?: ReactNode;
  ask?: ReactNode;
  asks?: readonly AskLine[];
  /** 무시하기 and 되돌리기 on one of `asks`, for the session they wait in; without it no question line is drawn. */
  onAsk?: (id: string, status: "open" | "dismissed") => void;
  foot?: ReactNode;
  /** The session's terminal; none for a session whose pane is gone. */
  onTerminal?: () => void;
} & ThreadPartActions) {
  const { t } = useInterfaceTranslation();
  useSessionThread(actions, paneId);

  // The core holds one thread; until it holds this pane's, this one is still being read.
  const thread = useShellStore((s) => (paneId && s.sessionThread?.pane_id === paneId ? s.sessionThread : null));
  const history = useShellStore((s) => s.sessionThreadHistory);
  const plans = useShellStore((s) => s.rest?.plans);
  const missions = useShellStore((s) => s.rest?.missions);
  const rows = useShellStore((s) => allAgents(s.agents, s.rest?.status?.remote));
  const refusal = useShellStore((s) => s.composerRefusal);
  const screen = useMemo(() => threadScreen(thread, history, { plans, missions }, rows), [thread, history, plans, missions, rows]);

  const [text, setText] = useState("");
  const [caret, setCaret] = useState(0);
  const box = useRef<HTMLTextAreaElement>(null);
  const tray = useComposerTray();
  // The request a send is waiting on and the tiles it carried; a change of model or effort keeps the words.
  const [sent, setSent] = useState<{ request: string; tiles: string[]; keepWords?: boolean } | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const words = (reason: string) => (INPUT_WORDS.has(reason) ? t(`desk.thread.input.${reason}` as MessageKey) : refusalText(reason, t));

  // The words stay in the box until the pane took them, so a refused send loses nothing.
  // The answer names its request, so it is read from whichever thread the core
  // holds: an input that arrived after another pane's thread opened is
  // answered there, as closed.
  const input = useShellStore((s) => s.sessionThread?.input ?? null);
  useEffect(() => {
    if (!sent || input?.request_id !== sent.request || input.state === "sending") return;
    setSent(null);
    if (input.state === "sent") {
      if (!sent.keepWords) setText("");
      // Only the files this send carried: one dropped since stays for the next.
      sent.tiles.forEach(tray.remove);
      setNotice(null);
    } else {
      setNotice(input.reason ? words(input.reason) : null);
    }
  }, [input, sent]);
  useEffect(() => {
    if (!sent || refusal?.request_id !== sent.request) return;
    setSent(null);
    setNotice(words(refusal.reason));
  }, [refusal, sent]);

  const held = threadInputHeld(thread);
  const send = () => {
    const files = tray.outgoing();
    if (!thread?.pane_id || held !== null || sent || !files) return;
    const request = submitThreadInput(thread.generation, thread.pane_id, text, files.map(({ name, bytes }) => ({ name, bytes })));
    if (request && typeof request === "object") {
      setNotice(words(request.refused));
    } else if (request) {
      // The bubble the record will show names these files by their stages, here and on a device.
      files.forEach((file, index) => keepSent(`${request}-${index}`, file.name, file.image));
      setSent({ request, tiles: files.map((file) => file.id) });
      setNotice(null);
    } else {
      setNotice(words("forward_failed"));
    }
  };
  // The `/` and `@` menus Desk's composer has: the session's agent's own commands, and the session's project's
  // files above Desk's sessions. A pick is written as the agent reads it in its terminal.
  const kind = rows.find((row) => row.pane_id === paneId)?.agent_kind ?? null;
  const adapter = kind ? agentAdapter(kind) : undefined;
  const lists = useShellStore((s) => s.deskCommands?.lists);
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
    commands: adapter ? { agent: adapter.picker_label, list: lists?.[adapter.agent_cli as keyof typeof lists] ?? null } : null,
    noList: "unknown",
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
    if (!thread?.pane_id || held !== null || sent) return;
    const request = actions.switchThread(thread.generation, thread.pane_id, to);
    if (request) {
      setSent({ request, tiles: [], keepWords: true });
      setNotice(null);
    } else {
      setNotice(words("forward_failed"));
    }
  };
  const bar = adapter && slots && !foot ? (
    <SessionAgentBar
      agent={{ kind: adapter.agent_cli, label: adapter.picker_label }}
      model={slots.model}
      effort={slots.effort}
      switchHint={{ model: switchCommand(adapter.switch?.model), effort: switchCommand(adapter.switch?.effort) }}
      held={held !== null || sent !== null}
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
    if (askedOlder.current || !thread?.has_older || (thread.older !== "idle" && thread.older !== "failed")) return;
    askedOlder.current = true;
    actions.readOlderThread(thread.generation);
  };

  const root = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (answer > 0) root.current?.querySelector<HTMLTextAreaElement>("[data-desk-composer-input]")?.focus();
  }, [answer]);

  if (!job && !foot) {
    // The thread opened from the first frame and its card is on its way, or the session has no card any more (its
    // pane closed): the head and its `✕` stand either way (B6).
    return (
      <div ref={root} className="flex min-h-0 min-w-0 flex-1 flex-col" data-desk-thread-panel={paneId ?? ""} data-desk-thread-pending="true">
        {head}
        <p role="status" className="p-md text-caption text-muted-foreground">{t("desk.thread.reading")}</p>
      </div>
    );
  }
  return (
    // The whole panel, its head, conversation and box, takes a dropped file into the box while the box stands (Doop L 나).
    <FileDropZone ref={root} tray={foot ? null : tray} className="flex min-h-0 min-w-0 flex-1 flex-col" data-desk-thread-panel={paneId ?? ""}>
      {head}
      <ThreadView
        key={paneId ?? ""}
        scrollKey={paneId ? `thread:${paneId}` : null}
        job={job}
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
        composer={{
          value: text,
          tray,
          focused,
          held: held !== null || sent !== null,
          notice: held === "pane_changed" || held === "pane_gone" ? words(held) : notice,
          textRef: box,
          menus,
          bar,
          onChange: (value, next) => {
            setText(value);
            setCaret(next);
          },
          onCaret: setCaret,
          onSend: send,
        }}
        onTerminal={onTerminal}
        onReachTop={reachTop}
        olderLeft={thread?.has_older ?? false}
        {...parts}
      />
    </FileDropZone>
  );
}
