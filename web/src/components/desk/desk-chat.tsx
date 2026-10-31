import { ArrowLeftRightIcon, ChevronRightIcon, CopyIcon, CornerDownLeftIcon, FilePenIcon, MailIcon, ShieldIcon, SlidersHorizontalIcon, SquareTerminalIcon, TerminalIcon } from "lucide-react";
import { useMemo, type ReactNode } from "react";
import { Trans } from "react-i18next";
import type { DeskAgent, DeskItem, DeskJob, DeskMention, DeskNoticeKind, DeskSessionRef, DeskSteps } from "../../desk";
import { chipMention, handedSession, markedMention, mentionParts, namedMention } from "../../desk";
import { AgentMark } from "../../AgentMark";
import { useInterfaceTranslation } from "../../i18n/client";
import { requireInterfaceLanguage } from "../../i18n/locale";
import type { MessageKey } from "../../i18n/catalogs";
import { cn } from "../../lib/utils";
import { MarkdownText, type MarkdownChip } from "../../MarkdownText";
import { Elapsed, formatElapsed } from "../elapsed";
import { MentionChip } from "../mention-chip";
import { StatusMark } from "../status-mark";
import { Button } from "../ui/button";
import { RowButton } from "../ui/row-button";
import { Hint } from "../ui/tooltip";
import { agentProductName } from "../../agentAdapters";
import { DeskJobCard, SessionChip, type DeskJobCardActions } from "./desk-job-card";

/**
 * Words with their mentions drawn as chips (B18, B26): a session and a
 * project read apart, a session whose state is known by its status and agent
 * marks (D-75); a chip with a press opens what it names. `token` is how the
 * words spell a mention: a session's own words say `@name`, a Desk message
 * holds `@session(<id> <name>)`.
 */
export function MentionText({ text, mentions, token = namedMention, onMention }: { text: string; mentions: readonly DeskMention[]; token?: (mention: DeskMention) => string; onMention?: (mention: DeskMention) => void }) {
  const { t } = useInterfaceTranslation();
  return <>{mentionParts(text, mentions, token).map((part, index) => {
    if (typeof part === "string") return part;
    const lead = part.kind === "session" && part.agent_kind ? <><StatusMark mark={part.mark ?? null} /><AgentMark kind={part.agent_kind} /></> : undefined;
    return onMention
      ? <MentionChip key={index} kind={part.kind} name={part.name} lead={lead} label={t("desk.mention.open", { name: part.name })} onPress={() => onMention(part)} />
      : <MentionChip key={index} kind={part.kind} name={part.name} lead={lead} />;
  })}</>;
}

/** The operator's own words, the one thing drawn as a bubble (B7, D-75). */
export function DeskUserMessage({ text, mentions, onMention }: { text: string; mentions: readonly DeskMention[]; onMention?: (mention: DeskMention) => void }) {
  return (
    <p className="max-w-(--size-pr-popover) self-end whitespace-pre-wrap break-keep rounded-xl bg-secondary px-md py-xs text-subhead leading-relaxed text-foreground [overflow-wrap:anywhere]" data-desk-item="user">
      <MentionText text={text} mentions={mentions} token={markedMention} onMention={onMention} />
    </p>
  );
}

/**
 * Desk's answer as plain Markdown (D-75), where a session it names is that
 * session's chip, pressed to open it; streaming while `live`; a stopped
 * answer keeps what it said and says it stopped (B7).
 */
export function DeskReply({ text, stopped = false, live = false, mentions = [], onMention }: { text: string; stopped?: boolean; live?: boolean; mentions?: readonly DeskMention[]; onMention?: (mention: DeskMention) => void }) {
  const { t } = useInterfaceTranslation();
  const chips = useMemo<MarkdownChip[]>(() => mentions.map((mention) => ({
    token: markedMention(mention),
    node: <SessionChip session={{ name: mention.name, mark: mention.mark ?? null, agent_kind: mention.agent_kind ?? null }} onPress={onMention ? () => onMention(mention) : undefined} />,
  })), [mentions, onMention]);
  return (
    <div className="flex min-w-0 flex-col gap-xxs" data-desk-item="reply" data-desk-reply={live ? "live" : stopped ? "stopped" : "done"}>
      <MarkdownText text={text} size="chat" chips={chips} />
      {stopped ? <span className="text-micro text-muted-foreground">{t("desk.reply.stopped")}</span> : null}
    </div>
  );
}

/** `생각 중 · 3초` from the moment a message is sent until the answer starts (B7): words alone, since a ring is always a session's state (D-75). */
export function DeskThinking({ startedAt = null }: { startedAt?: number | null }) {
  const { t } = useInterfaceTranslation();
  return (
    <p role="status" className="text-caption text-muted-foreground" data-desk-item="thinking">
      <span className="animate-pulse">{t("desk.thinking")}</span>
      {startedAt !== null ? <> · <Elapsed since={startedAt} /></> : null}
    </p>
  );
}

const STEP_WORDS = {
  sessions: "desk.steps.sessions",
  files: "desk.steps.files",
  letters: "desk.steps.letters",
  starts: "desk.steps.starts",
  other: "desk.steps.other",
} as const satisfies Record<keyof DeskSteps, string>;

/** A Desk turn folded into one line above its answer (D-75): how long it thought and what it did, counted; a turn that did nothing has none. */
export function DeskReplySteps({ thoughtMs, steps }: { thoughtMs: number | null; steps: DeskSteps }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const words = [
    thoughtMs !== null ? t("desk.steps.thought", { time: formatElapsed(language, thoughtMs) }) : null,
    ...(Object.keys(STEP_WORDS) as (keyof DeskSteps)[]).map((key) => (steps[key] > 0 ? t(STEP_WORDS[key], { count: steps[key] }) : null)),
  ].filter(Boolean);
  if (words.length === 0) return null;
  return <p className="text-caption text-muted-foreground" data-desk-reply-steps="true">{words.join(" · ")}</p>;
}

/** A line between messages that changes no state: the agent or its model changed (B8, B9). */
function MarkerLine({ icon, children, ...data }: { icon: ReactNode; children: ReactNode } & Record<`data-${string}`, string>) {
  return (
    <p className="flex min-w-0 items-center gap-sm text-micro text-muted-foreground" {...data}>
      <span aria-hidden="true" className="h-(--size-hairline) min-w-(--size-icon) flex-1 bg-border" />
      <span className="flex min-w-0 shrink items-center gap-xxs">
        {icon}
        <span className="truncate">{children}</span>
      </span>
      <span aria-hidden="true" className="h-(--size-hairline) min-w-(--size-icon) flex-1 bg-border" />
    </p>
  );
}

const NOTICE: Record<Extract<DeskItem, { kind: "notice" }>["notice"], MessageKey> = {
  preparing: "desk.notice.preparing",
  install_failed: "desk.notice.install_failed",
  login_needed: "desk.notice.login_needed",
  blocked: "desk.notice.blocked",
  host_stopped: "desk.notice.host_stopped",
  resume_failed: "desk.notice.resume_failed",
  no_ask_mode: "desk.notice.no_ask_mode",
  turn_failed: "desk.notice.turn_failed",
};

export type DeskNoticeAction = "retry" | "restart" | "reinstall";

/** The one action that answers a notice, from its kind. */
const NOTICE_ACTION: Partial<Record<DeskNoticeKind, DeskNoticeAction>> = {
  install_failed: "reinstall",
  blocked: "retry",
  host_stopped: "restart",
  turn_failed: "retry",
};

const ACTION_WORDS: Record<DeskNoticeAction, MessageKey> = {
  retry: "desk.action.retry",
  restart: "desk.action.restart",
  reinstall: "desk.action.reinstall",
};

const SWITCH_WORDS: Record<Extract<DeskItem, { kind: "switch" }>["carried"], MessageKey> = {
  all: "desk.switch.carried",
  trimmed: "desk.switch.trimmed",
  none: "desk.switch.none",
};

/**
 * A notice at the chat's end (B5, B6, B11, B13, B15), one line with no box
 * (D-75): what happened and the one action that answers it, as words. A
 * sign-in notice carries the command to run, copyable; the words the
 * operator sent stay in the composer.
 */
export function DeskNotice({ item, onAction, onCopy }: { item: Extract<DeskItem, { kind: "notice" }>; onAction: (action: DeskNoticeAction) => void; onCopy: (text: string) => void }) {
  const { t } = useInterfaceTranslation();
  const failed = item.notice === "blocked" || item.notice === "host_stopped" || item.notice === "install_failed" || item.notice === "turn_failed";
  const action = NOTICE_ACTION[item.notice];
  const words = t(NOTICE[item.notice], { agent: item.agent ? agentProductName(item.agent) : t("desk.name"), detail: item.detail ?? "" });
  return (
    <p className="flex min-w-0 flex-wrap items-center gap-x-sm gap-y-xxs text-caption" role={failed ? "alert" : "status"} data-desk-item="notice" data-desk-notice={item.notice}>
      <span className={cn("min-w-0", failed ? "text-warning" : item.notice === "preparing" ? "animate-pulse text-muted-foreground" : "text-subtle-foreground")}>{words}</span>
      {item.notice === "login_needed" && item.detail ? (
        <span className="inline-flex min-w-0 items-center gap-xxs rounded-xs bg-secondary px-xs font-mono text-micro text-foreground">
          <TerminalIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
          <span className="truncate">{item.detail}</span>
          <Hint label={t("desk.action.copy")}>
            <Button variant="ghost" size="icon-xs" aria-label={t("desk.action.copy")} onClick={() => onCopy(item.detail!)}><CopyIcon /></Button>
          </Hint>
        </span>
      ) : null}
      {action ? <Button variant="text-link" size="inline" className="text-caption" data-desk-notice-action={action} onClick={() => onAction(action)}>{t(ACTION_WORDS[action])}</Button> : null}
    </p>
  );
}

/** The answered line's mark: what the card asked to do. */
const APPROVAL_ICON: Record<Extract<DeskItem, { kind: "approval" }>["card"], typeof SquareTerminalIcon> = {
  command: SquareTerminalIcon,
  edit: FilePenIcon,
  other: ShieldIcon,
};

const APPROVAL_CARD: Record<Extract<DeskItem, { kind: "approval" }>["card"], MessageKey> = {
  command: "desk.approval.card.command",
  edit: "desk.approval.card.edit",
  other: "desk.approval.card.other",
};

/**
 * Desk asking before it runs a command or changes code (D-52, B14, D-75):
 * the one `!` card without an agent mark, since it is Desk's own. The
 * command whole on one grey block, the folder, Desk's reason and two grey
 * answers, neither the default. Once answered it folds to one dim line: how
 * it was answered and the command's start.
 */
export function DeskApprovalCard({ item, onAnswer }: { item: Extract<DeskItem, { kind: "approval" }>; onAnswer: (allow: boolean) => void }) {
  const { t } = useInterfaceTranslation();
  if (item.state !== "waiting") {
    const answered = item.state === "allowed" ? t("desk.approval.allowed") : item.state === "rejected" ? t("desk.approval.rejected") : t("desk.approval.closed");
    const Icon = APPROVAL_ICON[item.card];
    return (
      <p className="flex min-w-0 items-center gap-xs text-micro text-muted-foreground" title={item.command} data-desk-item="approval" data-desk-approval={item.state} data-desk-approval-answered={item.state}>
        <Icon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" data-desk-approval-card={item.card} />
        <span className="shrink-0">{answered}</span>
        <span aria-hidden="true">·</span>
        <span className="min-w-0 truncate font-mono">{item.command}</span>
      </p>
    );
  }
  return (
    <section className="flex min-w-0 flex-col gap-xs rounded-lg border border-border px-md py-sm" aria-label={t(APPROVAL_CARD[item.card])} data-desk-item="approval" data-desk-approval={item.state}>
      <p className="flex min-w-0 items-center gap-xs">
        <StatusMark mark="turn" />
        <span className="font-medium text-foreground">{t("desk.approval.title")}</span>
        <span className="min-w-0 truncate text-caption text-muted-foreground">{t(APPROVAL_CARD[item.card])}</span>
      </p>
      <pre className="min-w-0 whitespace-pre-wrap break-all rounded-sm bg-secondary px-sm py-xs font-mono text-caption text-foreground" data-desk-approval-command="true">{item.command}</pre>
      {item.folder ? (
        <p className="flex min-w-0 items-baseline gap-xs text-micro text-muted-foreground">
          <span className="shrink-0">{t("desk.approval.folder")}</span>
          <span className="min-w-0 truncate font-mono text-subtle-foreground" title={item.folder}>{item.folder}</span>
        </p>
      ) : null}
      {item.reason ? <p className="min-w-0 whitespace-normal break-keep text-caption text-subtle-foreground [overflow-wrap:anywhere]">{item.reason}</p> : null}
      <div className="flex items-center justify-end gap-xs pt-xxs">
        <Button variant="secondary" size="sm" data-desk-approval-answer="reject" onClick={() => onAnswer(false)}>{t("desk.approval.reject")}</Button>
        <Button variant="secondary" size="sm" data-desk-approval-answer="allow" onClick={() => onAnswer(true)}>{t("desk.approval.allow")}</Button>
      </div>
    </section>
  );
}

/**
 * A letter Desk sent a running session (D-24, B16), one line with no box
 * since it changes no state (D-75): the session's chip, `편지로 맡김` for
 * handed work and `보냄` for any other letter, its first line, and whether
 * the session took it in. The chip keeps the session's own status; pressing
 * it opens that session's thread.
 */
export function DeskHandedLine({ session, handed, line, read, onOpen }: { session: DeskSessionRef; handed: boolean; line: string; read: boolean; onOpen: () => void }) {
  const { t, i18n } = useInterfaceTranslation();
  return (
    <p className="flex min-w-0 items-center gap-xs text-caption text-muted-foreground" data-desk-item="handed" data-desk-handed={handed ? "request" : "letter"} data-desk-handed-read={read ? "true" : "false"}>
      <MailIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
      <span className="flex min-w-0 items-center whitespace-nowrap">
        <Trans i18n={i18n} t={t} i18nKey={handed ? "desk.handed.line" : "desk.handed.sent"} components={{ who: <SessionChip session={session} className="mr-xxs" onPress={onOpen} /> }} />
      </span>
      {line ? <><span aria-hidden="true">·</span><span className="min-w-0 truncate" title={line}>{line}</span></> : null}
      <span aria-hidden="true">·</span>
      <span className="shrink-0">{t(read ? "desk.handed.read" : "desk.handed.unread")}</span>
    </p>
  );
}

export type DeskChatActions = DeskJobCardActions & {
  onApproval: (id: number, allow: boolean) => void;
  onNoticeAction: (action: DeskNoticeAction, notice: Extract<DeskItem, { kind: "notice" }>) => void;
  onCopy: (text: string) => void;
  onMention: (mention: DeskMention) => void;
  onHanded: (paneId: string) => void;
};

const replyChips = new WeakMap<object, DeskMention[]>();

/** A reply's sessions as mentions, one array per reply value, so its chips are drawn once. */
function replyMentions(item: Extract<DeskItem, { kind: "reply" }>): DeskMention[] {
  let mentions = replyChips.get(item);
  if (!mentions) {
    mentions = item.mentions.map(chipMention);
    replyChips.set(item, mentions);
  }
  return mentions;
}

/** A job card's job: an item naming a job the snapshot does not carry is a broken snapshot, not an empty card. */
function jobOf(jobs: Readonly<Record<string, DeskJob>>, id: string): DeskJob {
  const job = jobs[id];
  if (!job) throw new Error(`Desk chat names an unknown job ${id}`);
  return job;
}

/**
 * Desk's conversation (D-17, D-39, D-75, B7-B16): the operator's words as
 * bubbles, Desk's answers as plain Markdown at the conversation's size, the
 * lines that change nothing, the approval card, the notices, and a card for
 * each job Desk started. `live` is the answer still arriving; `thinking`
 * shows until it starts.
 */
export function DeskChat({ items, jobs, agents, live = null, thinking = false, startedAt = null, selectedJob = null, selectedOutput = null, ...actions }: {
  items: readonly DeskItem[];
  jobs: Readonly<Record<string, DeskJob>>;
  /** The agents, whose choices name a model line's model and effort. */
  agents: readonly DeskAgent[];
  /** The answer streaming now, after the last item. */
  live?: string | null;
  thinking?: boolean;
  /** When the turn started (`desk_live.started_at_ms`), for `생각 중 · 3초`. */
  startedAt?: number | null;
  selectedJob?: string | null;
  selectedOutput?: string | null;
} & DeskChatActions) {
  const { t } = useInterfaceTranslation();
  // A model line holds the choice's id; its words are the choice's label, or the id an agent no longer offers.
  const choice = (id: string | null, of: "models" | "efforts") => id === null ? t("desk.bar.default") : (agents.flatMap((agent) => agent[of]).find((one) => one.id === id)?.label ?? id);
  return (
    <ol className="mx-auto flex w-full max-w-(--size-editor-measure) min-w-0 flex-col gap-md px-lg py-lg text-subhead" aria-label={t("desk.name")} data-desk-chat={items.length}>
      {items.map((item) => (
        <li key={item.id} className="flex min-w-0 flex-col">
          {item.kind === "user" ? <DeskUserMessage text={item.text} mentions={item.mentions} onMention={actions.onMention} />
            : item.kind === "reply" ? <div className="flex min-w-0 flex-col gap-xs"><DeskReplySteps thoughtMs={item.thought_ms} steps={item.steps} /><DeskReply text={item.text} stopped={item.state === "stopped"} mentions={replyMentions(item)} onMention={actions.onMention} /></div>
            : item.kind === "switch" ? <MarkerLine icon={<ArrowLeftRightIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />} data-desk-item="switch">{item.from === null ? t("desk.switch.start", { to: agentProductName(item.to) }) : t(SWITCH_WORDS[item.carried], { from: agentProductName(item.from), to: agentProductName(item.to) })}</MarkerLine>
            : item.kind === "model" ? <MarkerLine icon={<SlidersHorizontalIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />} data-desk-item="model">{t("desk.model", { model: choice(item.model, "models"), effort: choice(item.effort, "efforts") })}</MarkerLine>
            : item.kind === "approval" ? <DeskApprovalCard item={item} onAnswer={(allow) => actions.onApproval(item.id, allow)} />
            : item.kind === "notice" ? <DeskNotice item={item} onAction={(action) => actions.onNoticeAction(action, item)} onCopy={actions.onCopy} />
            : item.kind === "handed" ? <DeskHandedLine session={handedSession(item)} handed={item.letter_kind === "request"} line={item.line} read={item.read} onOpen={() => actions.onHanded(item.pane_id)} />
            : <DeskJobCard job={jobOf(jobs, item.job_id)} selected={selectedJob === item.job_id} selectedOutput={selectedOutput} {...actions} />}
        </li>
      ))}
      {live !== null ? <li className="flex min-w-0 flex-col"><DeskReply text={live} live /></li> : thinking ? <li><DeskThinking startedAt={startedAt} /></li> : null}
    </ol>
  );
}

/**
 * The empty Desk (D-68, B2, D-75): what Desk does in a line, three examples
 * as plain rows a press sends, and while threads run their real marks and
 * counts with the way to them. ◆ stays in the sidebar (D-38).
 */
export function DeskEmpty({ working, asking, onExample, onShowThreads }: { working: number; asking: number; onExample: (text: string) => void; onShowThreads?: () => void }) {
  const { t } = useInterfaceTranslation();
  const examples = [t("desk.empty.example1"), t("desk.empty.example2"), t("desk.empty.example3")];
  return (
    <div className="mx-auto flex w-full max-w-(--size-editor-measure) min-w-0 flex-1 flex-col justify-center gap-xs px-lg" data-desk-empty="true">
      <h2 className="text-title font-semibold text-foreground">{t("desk.empty.title")}</h2>
      <p className="whitespace-normal break-keep text-caption text-muted-foreground">{t("desk.empty.body")}</p>
      <ul className="mt-sm flex w-full min-w-0 flex-col">
        {examples.map((example) => (
          <li key={example}>
            <RowButton className="group/example rounded-md px-sm py-xs text-subhead text-subtle-foreground hover:text-foreground" data-desk-example="true" onClick={() => onExample(example)}>
              <span className="min-w-0 flex-1 truncate">{example}</span>
              <CornerDownLeftIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground opacity-0 group-hover/example:opacity-100 group-focus-visible/example:opacity-100" />
            </RowButton>
          </li>
        ))}
      </ul>
      {working + asking > 0 ? (
        <p className="mt-sm flex flex-wrap items-center gap-xs text-caption text-muted-foreground" data-desk-empty-live="true">
          {working > 0 ? <span className="flex items-center gap-xxs"><StatusMark mark="going" />{t("desk.empty.working", { count: working })}</span> : null}
          {working > 0 && asking > 0 ? <span aria-hidden="true">·</span> : null}
          {asking > 0 ? <span className="flex items-center gap-xxs text-warning"><StatusMark mark="turn" />{t("desk.empty.asking", { count: asking })}</span> : null}
          {onShowThreads ? (
            <>
              <span aria-hidden="true">·</span>
              <Button variant="text" size="inline" className="gap-0 text-caption" onClick={onShowThreads}>
                {t("desk.empty.showThreads")}
                <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" />
              </Button>
            </>
          ) : null}
        </p>
      ) : null}
    </div>
  );
}
