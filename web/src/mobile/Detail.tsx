import { OfflineText } from "./OfflineText";
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
import { ArrowLeftIcon, CircleAlertIcon, ImageIcon, MoonIcon, FlagIcon } from "lucide-react";
import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState, type UIEvent } from "react";
import { Elapsed } from "../components/elapsed";
import { useInterfaceTranslation } from "../i18n/translator";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { Tabs, TabsList, TabsTrigger } from "../components/ui/tabs";
import { wakeFailureReason, wakingLine } from "../sleep";
import type { AgentSleep } from "../snapshot";
import { renderMarkdown } from "./markdown";
import { AgentHead, DETAIL_INSET, PhoneHumanTurn, Place } from "./parts";
import { closeDetail, closeLook, loadMore, loadOlder, onReplyResult, openPrPage, sendKey, sendReply, setTab, wakeAgent } from "./connection";
import { LookViewer, LooksList, useKindWord } from "./Looks";
import {
  MAX_MESSAGES,
  MAX_REPLY_CHARS,
  QUICK_KEYS,
  boxDrawingRow,
  inputFailure,
  messageTime,
  noticeText,
  rowsProblem,
  type Conversation,
  type ConversationMessage,
  sleepTarget,
  replyAbout,
  type AgentKey,
  type DetailTab,
  type LookKind,
} from "./protocol";
import { agentOf, openMission, usePhone } from "./store";
import { agentLinks, progress } from "./missionModel";
import { TONE_TEXT } from "../components/mission-tone";
import { missionToneOf } from "../missions";
import { PR_LOOK } from "../prMark";
import { cn } from "../lib/utils";

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

/** The tab on screen: 볼 것 only while the agent has Views (or hided has not listed them yet) or one is open; else the half hided reads. */
function shownTab(detail: NonNullable<ReturnType<typeof usePhone.getState>["detail"]>, hasConversation: boolean): DetailTab {
  const empty = detail.looks !== null && detail.looks.rows.length === 0 && !detail.look;
  const tab = detail.tab === "looks" && empty ? detail.view : detail.tab;
  return tab === "conversation" && !hasConversation ? "terminal" : tab;
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
  const hasLooks = (detail?.looks?.rows.length ?? 0) > 0;
  const unseen = detail?.looks?.rows.filter((row) => row.unseen).length ?? 0;
  const tab = detail ? shownTab(detail, hasConversation) : "conversation";
  const viewing = tab === "looks" ? (detail?.look ?? null) : null;
  const showsConversation = tab === "conversation" && conversation !== "none";
  // A pane that left the list is gone for the reply bar too (B28).
  const gone = rows?.state === "gone" || (groups !== null && !agent);
  // The shell under a sleeping agent is not who the operator was talking to: its state stands in the terminal's place and nothing can be sent.
  const sleep = agent?.sleep ?? null;
  const rowsNote = tab !== "looks" && !showsConversation && rows ? rowsProblem(rows.state) : null;
  const note = rowsNote ?? (gone && (showsConversation || !rows || tab === "looks") ? rowsProblem("gone") : null);
  const offline = unreachable && !connected;
  return (
    <main className="flex h-full flex-col" data-phone-detail={detail ? `${detail.key.device_id}|${detail.key.pane_id}` : ""}>
      <header className="phone-safe-top shrink-0 border-b border-border bg-background px-lg pb-md">
        {/* Keep each label's width; wrap when the phone cannot fit all three items. */}
        <div className="flex flex-wrap items-center gap-sm pt-sm">
          <div className="flex min-w-max flex-1 basis-0">
            <Button variant="ghost" onClick={viewing ? closeLook : onBack} className="-ml-sm flex min-h-(--size-touch-target) items-center gap-xs px-sm text-body text-foreground" data-phone-back={viewing ? "looks" : "true"}>
              <ArrowLeftIcon aria-hidden="true" className="size-(--size-icon-lg)" />
              {viewing ? t("mobile.tab.looks") : t("mobile.list")}
            </Button>
          </div>
          {!viewing && (hasConversation || hasLooks) && detail && !sleep ? (
            <Tabs value={tab} onValueChange={(value) => setTab(value as DetailTab)} className="shrink-0">
              <TabsList>
                {hasConversation ? (
                  <TabsTrigger value="conversation" className="h-(--size-touch-target) px-sm text-body" data-phone-view="conversation">
                    {t("mobile.tab.conversation")}
                  </TabsTrigger>
                ) : null}
                <TabsTrigger value="terminal" className="h-(--size-touch-target) px-sm text-body" data-phone-view="terminal">
                  {t("mobile.tab.terminal")}
                </TabsTrigger>
                {hasLooks ? (
                  <TabsTrigger value="looks" className="h-(--size-touch-target) gap-xs px-sm text-body" data-phone-view="looks">
                    {t("mobile.tab.looks")}
                    {unseen > 0 ? (
                      <span className="inline-flex h-(--size-badge-height) min-w-(--size-badge-height) items-center justify-center rounded-full bg-warning px-xs font-mono text-micro text-status-foreground" data-phone-looks-unseen={unseen}>
                        {unseen}
                      </span>
                    ) : null}
                  </TabsTrigger>
                ) : null}
              </TabsList>
            </Tabs>
          ) : null}
          <span className="min-w-max flex-1 basis-0 text-right font-mono text-body text-muted-foreground">
            <Elapsed since={agent?.changed_at_unix_ms} />
          </span>
        </div>
        {viewing ? (
          <ViewingHead row={viewing.row} opener={agent?.title ?? ""} />
        ) : agent ? (
          <div className="mt-xs flex flex-col gap-xs">
            <AgentHead agent={agent} large />
            {detail ? <DetailLinks agentKey={detail.key} /> : null}
            <Place agent={agent} inset={DETAIL_INSET} />
            <PhoneHumanTurn agent={agent} inset={DETAIL_INSET} />
          </div>
        ) : null}
      </header>
      {offline ? (
        <div role="status" className="shrink-0 bg-secondary px-lg py-sm text-subhead text-subtle-foreground" data-phone-unreachable="true">
          <OfflineText />
        </div>
      ) : null}
      {note ? (
        <p role="status" className="shrink-0 px-lg py-sm text-subhead text-warning" data-phone-rows-state={rowsNote ? rows?.state : "gone"}>
          {noticeText(t, note)}
        </p>
      ) : null}
      {sleep && detail ? (
        <SleepState sleep={sleep} agentKey={detail.key} />
      ) : tab === "looks" && detail ? (
        // The last list and the last content stay under the unreachable line, dimmed (B45).
        <div className={`flex min-h-0 flex-1 flex-col ${offline ? "opacity-(--opacity-dimmed)" : ""}`} aria-busy={offline}>
          {viewing ? <LookViewer row={viewing.row} content={viewing.content} refreshing={viewing.refreshing} /> : <LooksList looks={detail.looks} />}
        </div>
      ) : !showsConversation ? (
        <Scrollback text={rows?.text ?? ""} more={rows?.more ?? false} lines={rows?.lines ?? 0} loading={!rows} />
      ) : conversation ? (
        <ConversationLog conversation={conversation} />
      ) : (
        <div className="min-h-0 flex-1 bg-background px-lg py-md">
          <p className="text-body text-muted-foreground">{t("mobile.loading")}</p>
        </div>
      )}
      <ReplyBar disabled={gone || !connected || sleep !== null} about={viewing?.row.name ?? null} keys={tab !== "looks"} />
    </main>
  );
}

/**
 * The missions and pull requests this agent belongs to, one chip each under its title: a mission's title with `n/m` for its
 * Conductor or the step's title for a worker, a pull request's state mark and `#number`. A chip closes the detail and opens that
 * page on its tab, whose back goes to that tab's list.
 */
function DetailLinks({ agentKey }: { agentKey: AgentKey }) {
  const { t } = useInterfaceTranslation();
  const missions = usePhone((s) => s.missions);
  const prs = usePhone((s) => s.prs?.items ?? null);
  const links = useMemo(() => agentLinks(agentKey, missions, prs), [agentKey, missions, prs]);
  if (links.missions.length === 0 && links.prs.length === 0) return null;
  // Each chip is a small pill inside a press area of a touch target's height, so a row of them stays light.
  const chip = "h-auto min-h-(--size-touch-target) max-w-full px-none py-none font-normal hover:bg-transparent";
  const pill = "flex min-w-0 items-center gap-xs rounded-full border border-border bg-secondary px-sm py-xxs text-body text-foreground";
  return (
    <div className={`-my-sm flex flex-wrap gap-x-sm ${DETAIL_INSET}`} data-phone-detail-links="true">
      {links.missions.map(({ mission, node, turn }) => {
        const { done, total } = progress(mission);
        return (
          <Button key={mission.id} variant="ghost" className={chip} data-phone-detail-mission={mission.id}
            aria-label={`${t("mobile.tab.missions")} · ${mission.title}`} onClick={() => { closeDetail(); openMission(mission.id); }}>
            <span className={pill}>
              <FlagIcon aria-hidden="true" className={cn("size-(--size-icon) shrink-0", TONE_TEXT[missionToneOf(mission.status, turn)])} />
              <span className="min-w-0 truncate">{mission.title}</span>
              <span aria-hidden="true" className="shrink-0 text-muted-foreground">·</span>
              {node ? <span className="min-w-0 truncate text-subtle-foreground">{node.title}</span>
                : <span className="shrink-0 font-mono text-subtle-foreground">{done}/{total}</span>}
            </span>
          </Button>
        );
      })}
      {links.prs.map((pr) => {
        const { icon: Icon, tone } = PR_LOOK[pr.state];
        return (
          <Button key={pr.key} variant="ghost" className={chip} data-phone-detail-pr={pr.key}
            aria-label={`#${pr.number} ${pr.title}`} onClick={() => { closeDetail(); openPrPage(pr.key); }}>
            <span className={pill}>
              <Icon aria-hidden="true" className={cn("size-(--size-pr-icon) shrink-0", tone)} />
              <span className="font-mono">#{pr.number}</span>
            </span>
          </Button>
        );
      })}
    </div>
  );
}

/** The head of a View on screen: its name, its kind, who opened it and how long ago (B13). */
function ViewingHead({ row, opener }: { row: { name: string; kind: LookKind; at: number }; opener: string }) {
  const { t } = useInterfaceTranslation();
  const word = useKindWord()(row.kind);
  return (
    <div className="mt-xs flex flex-col gap-xs" data-phone-viewing={row.name}>
      <h1 className="break-all text-title font-medium text-foreground">{row.name}</h1>
      <p className="flex min-w-0 flex-wrap items-center gap-x-sm text-body text-muted-foreground">
        {row.kind === "image" ? <ImageIcon aria-hidden="true" className="size-(--size-icon) shrink-0" /> : null}
        <span>{word}</span>
        {opener ? (
          <>
            <span aria-hidden="true">·</span>
            <span className="min-w-0 break-keep wrap-anywhere">{t("mobile.look.openedBy", { name: opener })}</span>
          </>
        ) : null}
        <span aria-hidden="true">·</span>
        <Elapsed since={row.at} className="font-mono" />
      </p>
    </div>
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
        <Button variant="secondary" className="mt-xs h-(--size-touch-target) px-lg text-title" disabled={blocked} data-phone-wake="wake" onClick={() => wakeAgent(agentKey)}>
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
 * Keeps the newest line in view while the reader is at the bottom, when lines
 * arrive and when the view itself grows or shrinks (the keyboard coming up
 * over the reply bar); a reader scrolled up stays where they were. When older
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
  useEffect(() => {
    const element = scroller.current;
    if (!element || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => {
      if (pinned.current) element.scrollTop = element.scrollHeight;
      heightBefore.current = element.scrollHeight;
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
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
      <pre aria-label={t("mobile.recentOutput")} className="phone-ansi m-none font-mono text-caption leading-normal text-foreground">
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
          <span className="min-w-0 whitespace-pre-wrap break-keep wrap-anywhere">{message.text}</span>
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
      <div className="space-y-sm break-keep wrap-anywhere text-body leading-normal text-foreground">
        {body}
        {message.truncated ? <p className="text-muted-foreground">{t("mobile.truncated")}</p> : null}
      </div>
      {time}
    </li>
  );
}

/**
 * `about` is the name of the View on screen: the reply leaves with it in front, in brackets (B30). `keys` is false on 볼 것, where
 * the quick keys answer nothing the operator is looking at.
 */
function ReplyBar({ disabled, about, keys }: { disabled: boolean; about: string | null; keys: boolean }) {
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
    <footer className="phone-safe-bottom shrink-0 border-t border-border bg-background px-lg pt-xs">
      {keys ? (
      <div className="grid grid-cols-5 gap-sm" role="group" aria-label={t("mobile.quickKeys")}>
        {QUICK_KEYS.map((quick) => (
          <Button
            key={quick.key}
            variant="ghost"
            aria-label={t(quick.name)}
            disabled={blocked}
            data-phone-key={quick.key}
            className="h-(--size-touch-target) rounded-md font-mono text-body"
            onClick={() => sendKey(quick.key)}
          >
            {quick.label}
          </Button>
        ))}
      </div>
      ) : null}
      <form
        className={`${keys ? "mt-sm" : "mt-xs"} flex gap-sm pb-md`}
        onSubmit={(event) => {
          event.preventDefault();
          sendReply(about ? replyAbout(about, text) : text);
        }}
      >
        <Input
          value={text}
          onChange={(event) => setText(event.target.value)}
          placeholder={about ? t("mobile.look.replyAbout", { name: about }) : t("mobile.replyPlaceholder")}
          aria-label={t("mobile.reply")}
          disabled={disabled}
          enterKeyHint="send"
          autoComplete="off"
          autoCorrect="off"
          data-phone-reply="true"
          aria-invalid={tooLong || undefined}
          className="h-(--size-touch-target) min-w-0 flex-1 rounded-lg border border-border bg-input px-md text-title text-foreground outline-none placeholder:text-muted-foreground focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-(--opacity-disabled)"
        />
        <Button variant="secondary" type="submit" disabled={blocked || text.trim().length === 0 || tooLong} className="h-(--size-touch-target) rounded-md px-md text-body font-medium" data-phone-send="true">
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
