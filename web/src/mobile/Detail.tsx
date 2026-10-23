// One agent's detail on the phone (PRD mobile-companion B24-B28, D-18): the
// row's head; the agent's conversation from its own transcript, the
// operator's messages as ❯ blocks and the agent's Markdown at full width, with
// older messages a pull to the top away; or the pane's recent rows read-only
// in the terminal's colours, wrapped at the phone's width because the pane is
// as wide as the desktop (a box-drawn rule is clipped to one line instead).
// Conversation | Terminal switches between them while the pane has a conversation; a
// pane without one shows its terminal alone. The quick keys and one-line
// reply are on every detail, whatever its group.

import { AnsiUp } from "ansi_up";
import { ArrowLeftIcon, CircleAlertIcon, MoonIcon } from "lucide-react";
import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState, type UIEvent } from "react";
import { Elapsed } from "../components/elapsed";
import { useInterfaceTranslation } from "../i18n/translator";
import { Button } from "../components/ui/button";
import { Tabs, TabsList, TabsTrigger } from "../components/ui/tabs";
import { wakeFailureReason, wakingLine } from "../sleep";
import type { AgentSleep } from "../snapshot";
import { renderMarkdown } from "./markdown";
import { AgentHead, DETAIL_INSET, Place } from "./parts";
import { loadMore, loadOlder, onReplyResult, sendKey, sendReply, setView, wakeAgent } from "./connection";
import {
  MAX_MESSAGES,
  MAX_REPLY_CHARS,
  QUICK_KEYS,
  UNREACHABLE_NOTICE,
  boxDrawingRow,
  inputFailure,
  messageTime,
  noticeText,
  rowsProblem,
  type Conversation,
  type ConversationMessage,
  sleepTarget,
  type AgentKey,
  type DetailView,
} from "./protocol";
import { agentOf, usePhone } from "./store";

const PULL_EDGE = 48;

type AnsiRow = { html: string; box: boolean };

/** One read's rows as HTML; a fresh converter, so no colour carries over from the last read. */
function ansiRows(text: string): AnsiRow[] {
  const ansi = new AnsiUp();
  ansi.use_classes = true;
  // Read-only rows: an OSC 8 link stays text, nothing on the page navigates away.
  ansi.url_allowlist = Object.create(null) as Record<string, number>;
  const rows = text.split("\n");
  if (rows.at(-1) === "") rows.pop();
  return rows.map((row) => {
    const html = ansi.ansi_to_html(row);
    return { html, box: boxDrawingRow(html.replace(/<[^>]*>/g, "")) };
  });
}

export function Detail({ onBack }: { onBack: () => void }) {
  const { t } = useInterfaceTranslation();
  const detail = usePhone((s) => s.detail);
  const groups = usePhone((s) => s.groups);
  const connected = usePhone((s) => s.connected);
  const unreachable = usePhone((s) => s.unreachable);
  const agent = agentOf(groups, detail?.key ?? null);
  const rows = detail?.rows ?? null;
  const conversation = detail?.conversation ?? null;
  const hasConversation = conversation !== null && conversation !== "none";
  const showsConversation = detail?.view === "conversation" && conversation !== "none";
  // A pane that left the list is gone for the reply bar too (B28).
  const gone = rows?.state === "gone" || (groups !== null && !agent);
  // The shell under a sleeping agent is not who the operator was talking to: its state stands in the terminal's place and nothing can be sent.
  const sleep = agent?.sleep ?? null;
  const rowsNote = !showsConversation && rows ? rowsProblem(rows.state) : null;
  const note = rowsNote ?? (gone && (showsConversation || !rows) ? rowsProblem("gone") : null);
  return (
    <main className="flex h-full flex-col" data-phone-detail={detail ? `${detail.key.device_id}|${detail.key.pane_id}` : ""}>
      <header className="phone-safe-top shrink-0 border-b border-border bg-card px-lg pb-md">
        {/* ← List, Conversation | Terminal in the middle, and the elapsed time share one row, so the switch costs no height. */}
        <div className="flex items-center gap-sm pt-sm">
          <div className="flex min-w-0 flex-1 basis-0">
            <button type="button" onClick={onBack} className="-ml-sm flex min-h-(--size-touch-target) items-center gap-xs px-sm text-title text-foreground" data-phone-back="true">
              <ArrowLeftIcon aria-hidden="true" className="size-(--size-icon-lg)" />
              {t("mobile.list")}
            </button>
          </div>
          {hasConversation && detail && !sleep ? (
            <Tabs value={detail.view} onValueChange={(value) => setView(value as DetailView)} className="shrink-0">
              <TabsList>
                <TabsTrigger value="conversation" className="h-(--size-touch-target) px-md text-title" data-phone-view="conversation">
                  {t("mobile.tab.conversation")}
                </TabsTrigger>
                <TabsTrigger value="terminal" className="h-(--size-touch-target) px-md text-title" data-phone-view="terminal">
                  {t("mobile.tab.terminal")}
                </TabsTrigger>
              </TabsList>
            </Tabs>
          ) : null}
          <span className="min-w-0 flex-1 basis-0 text-right font-mono text-body text-muted-foreground">
            <Elapsed since={agent?.changed_at_unix_ms} />
          </span>
        </div>
        {agent ? (
          <div className="mt-xs flex flex-col gap-xs">
            <AgentHead agent={agent} large />
            <Place agent={agent} inset={DETAIL_INSET} />
          </div>
        ) : null}
      </header>
      {unreachable && !connected ? (
        <div role="status" className="shrink-0 bg-secondary px-lg py-sm text-subhead text-subtle-foreground" data-phone-unreachable="true">
          {noticeText(t, UNREACHABLE_NOTICE)}
        </div>
      ) : null}
      {note ? (
        <p role="status" className="shrink-0 px-lg py-sm text-subhead text-warning" data-phone-rows-state={rowsNote ? rows?.state : "gone"}>
          {noticeText(t, note)}
        </p>
      ) : null}
      {sleep && detail ? (
        <SleepState sleep={sleep} agentKey={detail.key} />
      ) : !showsConversation ? (
        <Scrollback text={rows?.text ?? ""} more={rows?.more ?? false} lines={rows?.lines ?? 0} loading={!rows} />
      ) : conversation ? (
        <ConversationLog conversation={conversation} />
      ) : (
        <div className="min-h-0 flex-1 bg-background px-lg py-md">
          <p className="text-body text-muted-foreground">{t("mobile.loading")}</p>
        </div>
      )}
      <ReplyBar disabled={gone || !connected || sleep !== null} />
    </main>
  );
}

/**
 * What the detail shows in place of the terminal while the agent sleeps, in the
 * desktop pane's words (`SleepBody`): Sleeping with Wake agent, Waking…, or the
 * resume failure with its reason, Retry and Start new session.
 */
function SleepState({ sleep, agentKey }: { sleep: AgentSleep; agentKey: AgentKey }) {
  const { t } = useInterfaceTranslation();
  const connected = usePhone((s) => s.connected);
  const action = usePhone((s) => s.sleepAction);
  const mine = action?.sleepId === sleepTarget(agentKey) ? action : null;
  const blocked = !connected || !!mine?.requestId;
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-sm bg-background px-lg text-center" data-phone-sleep={sleep.state}>
      {sleep.state === "failed" ? (
        <>
          <CircleAlertIcon aria-hidden="true" className="size-(--size-icon-lg) text-destructive" />
          <p className="text-title text-foreground">{t("panes.sleep.resumeFailed")}</p>
          {sleep.reason ? (
            <p className="max-w-full text-body text-muted-foreground" role="alert">
              {wakeFailureReason(sleep, t)}
            </p>
          ) : null}
        </>
      ) : (
        <>
          <MoonIcon aria-hidden="true" className="size-(--size-icon-lg) text-muted-foreground" />
          <p className="text-title text-foreground" role={sleep.state === "waking" ? "status" : undefined}>
            {sleep.state === "waking" ? t("panes.sleep.waking") : t("panes.sleep.sleeping")}
          </p>
          {sleep.state === "waking" ? (
            <p className="text-body text-muted-foreground">{wakingLine(sleep, Date.now(), t)}</p>
          ) : sleep.progress ? (
            <p className="max-w-full truncate text-body text-muted-foreground">{sleep.progress}</p>
          ) : null}
        </>
      )}
      {sleep.state === "sleeping" ? (
        <Button className="mt-xs h-(--size-touch-target) px-lg text-title" disabled={blocked} data-phone-wake="wake" onClick={() => wakeAgent(agentKey)}>
          {t("panes.sleep.wake")}
        </Button>
      ) : null}
      {sleep.state === "failed" ? (
        <div className="mt-xs flex items-center gap-sm">
          <Button variant="secondary" className="h-(--size-touch-target) px-lg text-title" disabled={blocked} data-phone-wake="retry" onClick={() => wakeAgent(agentKey)}>
            {t("common.retry")}
          </Button>
          <Button variant="outline" className="h-(--size-touch-target) px-lg text-title" disabled={blocked} data-phone-wake="fresh" onClick={() => wakeAgent(agentKey, true)}>
            {t("panes.sleep.newSession")}
          </Button>
        </div>
      ) : null}
      {mine?.error ? (
        <p role="alert" className="text-body text-destructive" data-phone-wake-error="true">
          {t("panes.sleep.requestFailed")}
        </p>
      ) : null}
    </div>
  );
}

/**
 * Keeps the newest line in view while the reader is at the bottom; when older
 * lines arrive on top (`top` changes), keeps the line they were on where it
 * was (B24, B25). A scroll near the top calls `onTop`.
 */
function usePinnedScroll(content: unknown, top: number, onTop: () => void) {
  const scroller = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);
  const heightBefore = useRef(0);
  const topBefore = useRef(top);
  useLayoutEffect(() => {
    const element = scroller.current;
    if (!element) return;
    if (pinned.current) element.scrollTop = element.scrollHeight;
    else if (top !== topBefore.current) element.scrollTop += element.scrollHeight - heightBefore.current;
    heightBefore.current = element.scrollHeight;
    topBefore.current = top;
  }, [content, top]);
  const onScroll = (event: UIEvent<HTMLDivElement>) => {
    const element = event.currentTarget;
    pinned.current = element.scrollHeight - element.scrollTop - element.clientHeight < PULL_EDGE;
    heightBefore.current = element.scrollHeight;
    if (element.scrollTop < PULL_EDGE) onTop();
  };
  return { scroller, onScroll };
}

function Scrollback({ text, more, lines, loading }: { text: string; more: boolean; lines: number; loading: boolean }) {
  const { t } = useInterfaceTranslation();
  const rows = useMemo(() => ansiRows(text), [text]);
  const { scroller, onScroll } = usePinnedScroll(rows, lines, () => {
    if (more) loadMore();
  });
  return (
    <div ref={scroller} className="min-h-0 flex-1 overflow-auto overscroll-contain bg-background px-lg py-md" data-phone-scrollback={lines} onScroll={onScroll}>
      {more ? <p className="pb-sm text-center text-body text-muted-foreground">{t("mobile.pullOlder")}</p> : null}
      {loading ? <p className="text-body text-muted-foreground">{t("mobile.loading")}</p> : null}
      <pre aria-label={t("mobile.recentOutput")} className="phone-ansi m-none font-mono text-body leading-normal text-foreground">
        {rows.map((row, index) => (
          <span
            key={index}
            className={`block overflow-hidden ${row.box ? "whitespace-pre" : "whitespace-pre-wrap wrap-anywhere"}`}
            // ansi_up escapes the text; it adds only its own colour spans. A blank row keeps its line,
            // and a row's trailing padding in a background colour stops at the page's margin.
            dangerouslySetInnerHTML={{ __html: row.html || " " }}
          />
        ))}
      </pre>
    </div>
  );
}

function ConversationLog({ conversation }: { conversation: Conversation }) {
  const { t } = useInterfaceTranslation();
  const { messages, before } = conversation;
  const { scroller, onScroll } = usePinnedScroll(messages, messages[0]?.id ?? 0, loadOlder);
  const capped = before !== null && messages.length >= MAX_MESSAGES;
  return (
    <div ref={scroller} className="min-h-0 flex-1 overflow-auto overscroll-contain bg-background px-lg py-md" data-phone-conversation={messages.length} onScroll={onScroll}>
      {before !== null ? (
        <p className="pb-md text-center text-body text-muted-foreground" data-phone-older={capped ? "capped" : "more"}>
          {capped ? t("mobile.messageLimit", { limit: MAX_MESSAGES }) : t("mobile.pullOlder")}
        </p>
      ) : null}
      {messages.length === 0 ? <p className="text-body text-muted-foreground">{t("mobile.noConversation")}</p> : null}
      <ol aria-label={t("mobile.tab.conversation")} className="flex flex-col gap-md">
        {messages.map((message, index) => (
          // The time closes each turn: after the operator's message and after the agent's last one.
          <MessageItem key={message.id} message={message} timed={messages[index + 1]?.who !== message.who} />
        ))}
      </ol>
    </div>
  );
}

/** Memoized: a message never changes once it arrived, so its Markdown is parsed once. */
const MessageItem = memo(function MessageItem({ message, timed }: { message: ConversationMessage; timed: boolean }) {
  const { t } = useInterfaceTranslation();
  const time = timed && message.at_ms > 0 ? (
    <time dateTime={new Date(message.at_ms).toISOString()} className="block pt-xs text-right font-mono text-body text-muted-foreground">
      {messageTime(message.at_ms)}
    </time>
  ) : null;
  if (message.who === "stopped") {
    return (
      <li data-phone-message="stopped" className="text-body text-muted-foreground">
        {t("mobile.stopped")}
      </li>
    );
  }
  if (message.who === "you") {
    return (
      <li data-phone-message="you">
        <p className="flex gap-sm rounded-md bg-secondary px-md py-sm text-body leading-normal text-foreground">
          <span aria-hidden="true" className="text-muted-foreground">
            ❯
          </span>
          <span className="min-w-0 whitespace-pre-wrap wrap-anywhere">{message.text}</span>
        </p>
        {time}
      </li>
    );
  }
  return <AgentMessage message={message} time={time} />;
});

function AgentMessage({ message, time }: { message: ConversationMessage; time: React.ReactNode }) {
  const { t } = useInterfaceTranslation();
  const body = useMemo(() => renderMarkdown(message.text), [message.text]);
  return (
    <li data-phone-message="agent">
      <div className="space-y-sm wrap-anywhere text-body leading-normal text-foreground">
        {body}
        {message.truncated ? <p className="text-muted-foreground">{t("mobile.truncated")}</p> : null}
      </div>
      {time}
    </li>
  );
}

function ReplyBar({ disabled }: { disabled: boolean }) {
  const { t } = useInterfaceTranslation();
  const [text, setText] = useState("");
  const pending = usePhone((s) => s.pendingInput);
  const error = usePhone((s) => s.inputError);
  useEffect(
    () =>
      onReplyResult((ok) => {
        if (ok) setText("");
      }),
    [],
  );
  const tooLong = [...text].length > MAX_REPLY_CHARS;
  const blocked = disabled || pending !== null;
  const shown = tooLong ? inputFailure("too_long") : error;
  return (
    <footer className="phone-safe-bottom shrink-0 border-t border-border bg-card px-lg pt-md">
      <div className="grid grid-cols-5 gap-sm" role="group" aria-label={t("mobile.quickKeys")}>
        {QUICK_KEYS.map((quick) => (
          <Button
            key={quick.key}
            variant="secondary"
            aria-label={t(quick.name)}
            disabled={blocked}
            data-phone-key={quick.key}
            className="h-(--size-touch-target) rounded-lg font-mono text-title"
            onClick={() => sendKey(quick.key)}
          >
            {quick.label}
          </Button>
        ))}
      </div>
      <form
        className="mt-sm flex gap-sm pb-md"
        onSubmit={(event) => {
          event.preventDefault();
          sendReply(text);
        }}
      >
        <input
          value={text}
          onChange={(event) => setText(event.target.value)}
          placeholder={t("mobile.replyPlaceholder")}
          aria-label={t("mobile.reply")}
          disabled={disabled}
          enterKeyHint="send"
          autoComplete="off"
          autoCorrect="off"
          data-phone-reply="true"
          aria-invalid={tooLong || undefined}
          className="h-(--size-touch-target) min-w-0 flex-1 rounded-lg border border-border bg-input px-md text-title text-foreground outline-none placeholder:text-muted-foreground focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50"
        />
        <Button type="submit" disabled={blocked || text.trim().length === 0 || tooLong} className="h-(--size-touch-target) rounded-lg px-lg text-title font-semibold" data-phone-send="true">
          {t("mobile.send")}
        </Button>
      </form>
      {shown ? (
        <p role="alert" className="-mt-sm pb-md text-body text-destructive" data-phone-input-error="true">
          {noticeText(t, shown)}
        </p>
      ) : null}
    </footer>
  );
}
