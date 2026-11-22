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
import { CircleAlertIcon, ImageIcon, MoonIcon, FlagIcon } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type UIEvent } from "react";
import { Elapsed, useElapsed } from "../components/elapsed";
import { useInterfaceTranslation } from "../i18n/translator";
import { Button } from "../components/ui/button";
import { Tabs, TabsList, TabsTrigger } from "../components/ui/tabs";
import { wakeFailureReason, wakingLine } from "../sleep";
import type { AgentSleep } from "../snapshot";
import { AgentLead, PhoneHumanTurn, PlaceParts } from "./parts";
import { ActivityLine, Composer, PageHead, StatusStrip } from "./layout";
import { lineParts } from "../agentRow";
import { statusText } from "../agentStatus";
import { closeDetail, closeLook, loadMore, loadOlder, onReplyResult, openPrPage, sendChoice, sendKey, sendReply, setTab, wakeAgent } from "./connection";
import { LookViewer, LooksList, useKindWord } from "./Looks";
import { AskCardView, AskStrip } from "./AskCard";
import { MessageItem } from "./messages";
import { AttachButton, ComposerFiles, FileNoticeLine } from "./Attachments";
import { attachPicked, pickFiles } from "./pickFiles";
import { removeFile, retryFile, shownFiles } from "./uploads";
import {
  MAX_MESSAGES,
  askIdentity,
  QUICK_KEYS,
  boxDrawingRow,
  inputFailure,
  noticeText,
  rowsProblem,
  type Conversation,
  type PhoneAsk,
  type Running,
  sleepTarget,
  replyAbout,
  replyProblem,
  type AgentKey,
  type DetailTab,
  type LookKind,
  type PhoneAgent,
} from "./protocol";
import { agentOf, openMission, usePhone } from "./store";
import { agentLinks, progress } from "./missionModel";
import { TONE_TEXT } from "../components/mission-tone";
import { missionStalled, missionToneOf } from "../missions";
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
  // Whether a reply is being written: the ask folds to one line above it then (Doop 06).
  const [composing, setComposing] = useState(false);
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
      {viewing ? (
        <ViewingHead row={viewing.row} opener={agent?.title ?? ""} />
      ) : (
        <PageHead
          back={{ label: t("mobile.list"), onClick: onBack, tag: "true" }}
          lead={agent ? <AgentLead agent={agent} /> : null}
          title={agent?.title ?? ""}
          line={agent ? <PlaceParts agent={agent} /> : null}
        >
          {(hasConversation || hasLooks) && detail && !sleep ? (
            <div className="mt-sm flex min-w-0 flex-wrap items-center justify-between gap-sm">
              <Tabs value={tab} onValueChange={(value) => setTab(value as DetailTab)} className="shrink-0">
                <TabsList>
                  {hasConversation ? (
                    <TabsTrigger value="conversation" className="h-(--size-touch-target) px-md text-body" data-phone-view="conversation">
                      {t("mobile.tab.conversation")}
                    </TabsTrigger>
                  ) : null}
                  <TabsTrigger value="terminal" className="h-(--size-touch-target) px-md text-body" data-phone-view="terminal">
                    {t("mobile.tab.terminal")}
                  </TabsTrigger>
                  {hasLooks ? (
                    <TabsTrigger value="looks" className="h-(--size-touch-target) gap-xs px-md text-body" data-phone-view="looks">
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
              <DetailLinks agentKey={detail.key} />
            </div>
          ) : detail ? <DetailLinks agentKey={detail.key} className="mt-sm" /> : null}
          {agent ? <PhoneHumanTurn agent={agent} inset="mt-sm" /> : null}
        </PageHead>
      )}
      {agent && !viewing && !sleep ? <AgentStrip agent={agent} /> : null}
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
        <Scrollback text={rows?.text ?? ""} more={rows?.more ?? false} lines={rows?.lines ?? 0} loading={!rows} alternateScreen={rows?.alternateScreen ?? false} conversation={hasConversation} />
      ) : conversation ? (
        <ConversationLog conversation={conversation} agent={agent ?? null} ask={composing ? null : (detail?.ask ?? null)} running={detail?.running ?? null} disabled={gone || !connected || sleep !== null} />
      ) : (
        <div className="min-h-0 flex-1 bg-background px-lg py-md">
          <p className="text-body text-muted-foreground">{t("mobile.loading")}</p>
        </div>
      )}
      <ReplyBar
        disabled={gone || !connected || sleep !== null}
        about={viewing?.row.name ?? null}
        keys={!showsConversation && tab !== "looks"}
        ask={showsConversation ? (detail?.ask ?? null) : null}
        onComposing={setComposing}
      />
    </main>
  );
}

/**
 * The missions and pull requests this agent belongs to, one chip each under its title: a mission's title with `n/m` for its
 * Conductor or the step's title for a worker, a pull request's state mark and `#number`. A chip closes the detail and opens that
 * page on its tab, whose back goes to that tab's list.
 */
/** The chips of what the agent belongs to; nothing at all, not even its room, when it belongs to nothing. */
function DetailLinks({ agentKey, className }: { agentKey: AgentKey; className?: string }) {
  const { t } = useInterfaceTranslation();
  const missions = usePhone((s) => s.missions);
  const prs = usePhone((s) => s.prs?.items ?? null);
  const links = useMemo(() => agentLinks(agentKey, missions, prs), [agentKey, missions, prs]);
  if (links.missions.length === 0 && links.prs.length === 0) return null;
  // Each chip is a small pill inside a press area of a touch target's height, so a row of them stays light.
  const chip = "h-auto min-h-(--size-touch-target) max-w-full px-none py-none font-normal hover:bg-transparent";
  const pill = "flex min-w-0 items-center gap-xs rounded-full bg-secondary px-md py-xs text-body text-foreground";
  return (
    <div className={cn("-my-sm flex min-w-0 flex-wrap justify-end gap-x-sm", className)} data-phone-detail-links="true">
      {links.missions.map(({ mission, node, turn }) => {
        const { done, total } = progress(mission);
        return (
          <Button key={mission.id} variant="ghost" className={chip} data-phone-detail-mission={mission.id}
            aria-label={[t("mobile.tab.missions"), mission.title, node?.title, `${done}/${total}`].filter(Boolean).join(" · ")} onClick={() => { closeDetail(); openMission(mission.id); }}>
            {/* The flag and the mission's progress, as Doop 02 draws it; the mission's and the step's names are the chip's name. */}
            <span className={pill}>
              <FlagIcon aria-hidden="true" className={cn("size-(--size-icon) shrink-0", TONE_TEXT[missionToneOf(mission.status, turn, missionStalled(mission))])} />
              <span className="shrink-0 font-mono" data-phone-detail-mission-progress={`${done}/${total}`}>{done}/{total}</span>
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

/** The head of a View on screen (Doop 11): back to 볼 것, its name, then its kind, the agent that opened it and how long ago (B13). */
function ViewingHead({ row, opener }: { row: { name: string; kind: LookKind; at: number }; opener: string }) {
  const { t } = useInterfaceTranslation();
  const word = useKindWord()(row.kind);
  return (
    <PageHead
      back={{ label: t("mobile.tab.looks"), onClick: closeLook, tag: "looks" }}
      title={<span className="break-all">{row.name}</span>}
      line={
        <>
          {row.kind === "image" ? <ImageIcon aria-hidden="true" className="size-(--size-icon) shrink-0" /> : null}
          <span>{word}</span>
          {opener ? (
            <>
              <span aria-hidden="true">·</span>
              <span className="sr-only">{t("mobile.look.openedBy", { name: opener })}</span>
              <span aria-hidden="true" className="min-w-0 break-keep wrap-anywhere" data-phone-viewing-opener="true">{opener}</span>
            </>
          ) : null}
          <span aria-hidden="true">·</span>
          <Elapsed since={row.at} className="font-mono" />
        </>
      }
      data-phone-viewing={row.name}
    />
  );
}

/** Where the agent stands, under the head (Doop 02, 09, 10): the state's word in its mark's colour, the row's line and how long ago it changed. */
function AgentStrip({ agent }: { agent: PhoneAgent }) {
  const { t } = useInterfaceTranslation();
  // The word that leads the row's line (`작업 중`, `자식 2개 기다림`) is the strip's word, so it is said once.
  const parts = agent.line ? lineParts(t, agent.wait === "children" && agent.wait_count === undefined ? { ...agent, wait_count: agent.descendants_working } : agent, agent.line) : null;
  const line = parts ? [parts.text, parts.tail].filter(Boolean).join(" · ") || null : null;
  // An agent asking the operator reads as their turn, not as the core's idle.
  const word = parts?.lead ?? (agent.mark === "turn" ? t("agents.mark.turn") : statusText(t, agent.status_code));
  return <StatusStrip word={word} tone={STRIP_TONE[agent.mark] ?? "text-muted-foreground"} line={line} since={agent.changed_at_unix_ms} />;
}

/** The strip's word takes the colour of the row's mark. */
const STRIP_TONE: Partial<Record<PhoneAgent["mark"], string>> = {
  going: "text-agent-working",
  waiting: "text-agent-working",
  turn: "text-warning",
  look: "text-warning",
  done: "text-success",
};

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

function Scrollback({ text, more, lines, loading, alternateScreen, conversation }: { text: string; more: boolean; lines: number; loading: boolean; alternateScreen: boolean; conversation: boolean }) {
  const { t } = useInterfaceTranslation();
  const rows = useMemo(() => ansiRows(text), [text]);
  const { scroller, onScroll } = usePinnedScroll(rows, lines, () => {
    if (more) loadMore();
  });
  return (
    <div ref={scroller} className="min-h-0 flex-1 overflow-auto overscroll-contain bg-background px-lg py-md" data-phone-scrollback={lines} onScroll={onScroll}>
      {more ? <p className="pb-sm text-center text-body text-muted-foreground">{t("mobile.pullOlder")}</p> : null}
      {alternateScreen && !more ? (
        conversation ? (
          <Button variant="text" size="inline" className="mb-sm min-h-(--size-touch-target) w-full whitespace-normal break-keep wrap-anywhere text-left text-body text-muted-foreground" data-phone-screen-history onClick={() => setTab("conversation")}>
            {t("mobile.screenHistory")}
          </Button>
        ) : <p className="pb-sm break-keep wrap-anywhere text-body text-muted-foreground" data-phone-screen-history>{t("mobile.screenHistory")}</p>
      ) : null}
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

function ConversationLog({ conversation, agent, ask, running, disabled }: { conversation: Conversation; agent: PhoneAgent | null; ask: PhoneAsk | null; running: Running | null; disabled: boolean }) {
  const { t } = useInterfaceTranslation();
  const { messages, before } = conversation;
  // While the agent works, the foot of its conversation says it runs a command, by the clock from when it started (Doop 02), or
  // that it thinks, counted from its last word or its last change (Doop 09).
  const lastAt = messages.at(-1)?.at_ms ?? 0;
  const working = agent?.status_code === "working";
  const commandSince = working && running?.kind === "command" ? running.since_ms : null;
  // An unknown running line claims neither: a command may run among the calls the computer did not keep.
  const thinkingSince = working && commandSince === null && running?.kind !== "unknown" ? Math.max(lastAt, agent.changed_at_unix_ms ?? 0) || null : null;
  // The activity line, the ask and its answer are content too: one that appears or grows with no new word, such as a refusal
  // under the card, still pulls a reader at the bottom down to it.
  const thinking = thinkingSince !== null || commandSince !== null;
  const key = usePhone((s) => s.detail?.key ?? null);
  const answered = usePhone((s) => (key && ask ? s.answers[askIdentity(key, ask)] : undefined)) ?? null;
  const content = useMemo(() => ({ messages, thinking, ask, answered }), [messages, thinking, ask, answered]);
  const { scroller, onScroll } = usePinnedScroll(content, messages[0]?.id ?? 0, loadOlder);
  // The operator's messages that attached files: the files over the bubble, with this phone's thumbnails where it sent them (Doop 09).
  const shown = useMemo(
    () =>
      messages.map((message) => {
        const attached = key && message.who === "you" ? shownFiles(key, message.text) : null;
        return attached && attached.files.length > 0 ? { message: { ...message, text: attached.words }, files: attached.files } : { message, files: undefined };
      }),
    [messages, key],
  );
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
          <MessageItem key={message.id} message={shown[index]?.message ?? message} files={shown[index]?.files} timed={messages[index + 1]?.who !== message.who} />
        ))}
      </ol>
      {commandSince ? <ActivityLine word={t("mobile.runningCommand")} since={commandSince} clock /> : null}
      {thinkingSince ? <ActivityLine word={t("mobile.thinking")} since={thinkingSince} /> : null}
      {ask ? <AskCard ask={ask} disabled={disabled} /> : null}
    </div>
  );
}

/** The card on the phone: the choice marked here is kept for its turn, and 답하기 sends it to the computer (`sendChoice`). */
export function AskCard({ ask, disabled }: { ask: PhoneAsk; disabled: boolean }) {
  const { t } = useInterfaceTranslation();
  const key = usePhone((s) => s.detail?.key ?? null);
  const identity = key ? askIdentity(key, ask) : null;
  const answered = usePhone((s) => (identity ? s.answers[identity] : undefined)) ?? null;
  const [marked, setMarked] = useState<{ identity: string; id: string } | null>(null);
  const waited = useElapsed(ask.since_ms);
  // The answer sent for this ask stands for the mark too, so a card drawn again (the strip unfolded, the detail opened again)
  // still shows what was sent.
  return (
    <AskCardView
      ask={ask}
      disabled={disabled}
      waited={waited}
      picked={marked && marked.identity === identity ? marked.id : (answered?.choice ?? null)}
      answer={answered?.state ?? null}
      error={answered?.state === "failed" ? noticeText(t, answered.notice) : null}
      inTerminal={answered?.state === "failed" && answered.inTerminal}
      onPick={(id) => identity && setMarked({ identity, id })}
      onAnswer={sendChoice}
      onTerminal={() => setTab("terminal")}
    />
  );
}

/**
 * `about` is the name of the View on screen: the reply leaves with it in front, in brackets (B30). `keys` is true on the terminal
 * alone (Doop 05), where the operator sees what a key does; the conversation answers an ask with its card, and 볼 것 with words.
 */
function ReplyBar({ disabled, about, keys, ask, onComposing }: { disabled: boolean; about: string | null; keys: boolean; ask: PhoneAsk | null; onComposing: (composing: boolean) => void }) {
  const { t } = useInterfaceTranslation();
  const [text, setText] = useState("");
  const [composing, setComposingHere] = useState(false);
  const setComposing = (next: boolean) => {
    setComposingHere(next);
    onComposing(next);
  };
  const pending = usePhone((s) => s.pendingInput);
  const error = usePhone((s) => s.inputError);
  // A computer that takes files offers the `+`; its files go up as soon as they are picked (Doop 08).
  const takesFiles = usePhone((s) => s.takesFiles);
  const files = usePhone((s) => s.files);
  const fileNotice = usePhone((s) => s.fileNotice);
  const filesReady = files.every((file) => file.state === "ready");
  // A computer that answers from the phone takes a reply of several lines, as long as one phone message holds.
  const lines = usePhone((s) => s.answer);
  const agentKey = usePhone((s) => s.detail?.key ?? null);
  useEffect(
    () =>
      onReplyResult((ok) => {
        if (ok) setText("");
      }),
    [],
  );
  const reply = about ? replyAbout(about, text) : text;
  const tooLong = useMemo(() => (agentKey ? replyProblem(reply, lines, agentKey, files.length) === "too_long" : false), [reply, lines, agentKey, files.length]);
  const blocked = disabled || pending !== null;
  const shown = tooLong ? inputFailure("too_long", lines) : error;
  return (
    <footer className="phone-safe-bottom shrink-0 border-t border-border bg-background px-lg pt-xs">
      {ask && composing ? <div className="-mx-lg -mt-xs mb-xs"><AskStrip ask={ask} /></div> : null}
      {keys ? (
      <div className="grid grid-cols-5 gap-sm" role="group" aria-label={t("mobile.quickKeys")}>
        {QUICK_KEYS.map((quick) => (
          <Button
            key={quick.key}
            variant="secondary"
            aria-label={t(quick.name)}
            disabled={blocked}
            data-phone-key={quick.key}
            className="h-(--size-touch-target) min-w-0 rounded-md px-xs font-mono text-body"
            onClick={() => sendKey(quick.key)}
          >
            {quick.label}
          </Button>
        ))}
      </div>
      ) : null}
      <div className={`${keys ? "mt-sm" : "mt-xs"} pb-md`}>
        {takesFiles && !about ? (
          <>
            <ComposerFiles files={files} onRemove={removeFile} onRetry={retryFile} />
            <FileNoticeLine notice={fileNotice} />
          </>
        ) : null}
        <Composer
          value={text}
          onChange={setText}
          onSubmit={() => sendReply(reply)}
          lines={lines}
          placeholder={
            about ? t("mobile.look.replyAbout", { name: about }) : keys ? t("mobile.replyToTerminal") : ask?.kind === "question" ? t("mobile.ask.replyInstead") : t("mobile.replyPlaceholder")
          }
          disabled={disabled}
          lead={takesFiles && !about ? <AttachButton disabled={disabled} onPick={() => void pickFiles().then(attachPicked)} /> : undefined}
          sendDisabled={blocked || (text.trim().length === 0 && files.length === 0) || !filesReady || tooLong}
          input={{
            "aria-label": t("mobile.reply"),
            "aria-invalid": tooLong || undefined,
            "data-phone-reply": "true",
            onFocus: () => setComposing(true),
            onBlur: () => setComposing(false),
          }}
          send={{ "data-phone-send": "true" }}
        />
      </div>
      {shown ? (
        <p role="alert" className="-mt-sm pb-md text-body text-destructive" data-phone-input-error="true">
          {noticeText(t, shown)}
        </p>
      ) : null}
    </footer>
  );
}
