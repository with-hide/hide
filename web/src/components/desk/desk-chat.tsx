import { ArrowLeftRightIcon, ChevronDownIcon, ChevronUpIcon, CopyIcon, CornerDownLeftIcon, FilePenIcon, FileSearchIcon, HouseIcon, MailIcon, ShieldIcon, SlidersHorizontalIcon, SquareTerminalIcon, TerminalIcon, XIcon } from "lucide-react";
import { useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Trans } from "react-i18next";
import type { ApprovalButton, ApprovalWord, DeskAgent, DeskApproval, DeskItem, DeskJob, DeskMention, DeskMissionMention, DeskNoticeKind, WorkingPhase, DeskQueuedMessage, DeskSessionRef, DeskSteps } from "../../desk";
import { approvalButtons, chipMention, DESK_REFUSAL_MAX, failureReason, handedSession, markedMention, markedMission, mentionParts, namedMention, namedText, refuses } from "../../desk";
import { sentAttachment } from "../../deskComposer";
import { useInterfaceTranslation } from "../../i18n/client";
import type { DeskAttachment } from "../../snapshot";
import { requireInterfaceLanguage } from "../../i18n/locale";
import type { MessageKey } from "../../i18n/catalogs";
import { composingKey } from "../../keys";
import { cn } from "../../lib/utils";
import { MarkdownText, type MarkdownChip } from "../../MarkdownText";
import { CopyableMessage } from "../copy-words";
import type { CopyText } from "../human-turn-copy";
import { formatElapsed } from "../elapsed";
import { MentionChip } from "../mention-chip";
import { MissionChipOf } from "../mission-chip";
import { useShellStore } from "../../store";
import { localDeviceId } from "../../snapshot";
import type { AnswerPlace, FoundAnswerPath } from "../../answerPaths";
import { useFilePathChips } from "./file-path-chip";
import { StatusMark } from "../status-mark";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { RowButton } from "../ui/row-button";
import { Hint } from "../ui/tooltip";
import { agentProductName } from "../../agentAdapters";
import { DeskJobCard, type DeskJobCardActions } from "./desk-job-card";
import { SessionChip, useSessionChipBridge } from "./session-chip";
import { SentAttachments } from "./desk-composer";
import { WorkingLine } from "./working-line";

/**
 * Words with their mentions drawn as chips (B18, B26): a session and a
 * project read apart, a session whose state is known by its status and agent
 * marks (D-75); a chip with a press opens what it names. `token` is how the
 * words spell a mention: a session's own words say `@name`, a Desk message
 * holds `@session(<id> <name>)`.
 */
export function MentionText({ text, mentions, token = namedMention, onMention }: { text: string; mentions: readonly DeskMention[]; token?: (mention: DeskMention) => string; onMention?: (mention: DeskMention) => void }) {
  return <>{mentionParts(text, mentions, token).map((part, index) => (typeof part === "string" ? part : <DeskMentionChip key={index} mention={part} onPress={onMention ? () => onMention(part) : undefined} />))}</>;
}

/**
 * One mention as Desk draws it wherever it stands, in the composer, the
 * operator's message and a thread alike (D-85): a session is its
 * `SessionChip`, the same one a reply names, and a project a green chip led
 * by ⌂.
 */
export function DeskMentionChip({ mention, onPress }: { mention: DeskMention; onPress?: () => void }) {
  const { t } = useInterfaceTranslation();
  if (mention.kind === "session") return <SessionChip session={{ name: mention.name, mark: mention.mark ?? null, agent_kind: mention.agent_kind ?? null, pane_id: mention.id }} onPress={onPress} />;
  const lead = <HouseIcon aria-hidden="true" className="size-(--size-icon-sm)" />;
  return onPress
    ? <MentionChip kind="project" name={mention.name} lead={lead} label={t("desk.mention.open", { name: mention.name })} onPress={onPress} />
    : <MentionChip kind="project" name={mention.name} lead={lead} />;
}

/** The operator's own words, the one thing drawn as a bubble (B7, D-75), its files above it, with `복사` on hover when `copy` is given (Doop E2). */
export function DeskUserMessage({ text, mentions, attachments = [], onMention, copy }: { text: string; mentions: readonly DeskMention[]; attachments?: readonly DeskAttachment[]; onMention?: (mention: DeskMention) => void; copy?: CopyText }) {
  const files = useMemo(() => attachments.map((file) => ({ ...sentAttachment(file.path, file.bytes), name: file.name })), [attachments]);
  const bubble = text ? (
    <p className="max-w-(--size-pr-popover) whitespace-pre-wrap break-keep rounded-xl bg-secondary px-md py-xs text-subhead leading-relaxed text-foreground [overflow-wrap:anywhere]">
      <MentionText text={text} mentions={mentions} token={markedMention} onMention={onMention} />
    </p>
  ) : null;
  return (
    <div className="flex min-w-0 flex-col items-end gap-xs" data-desk-item="user">
      <SentAttachments files={files} />
      {bubble && copy ? <CopyableMessage text={namedText(text, mentions)} copy={copy} align="end">{bubble}</CopyableMessage> : bubble}
    </div>
  );
}

/**
 * A message sent while a reply was being written, waiting for the turn before
 * it (stopped, with `다시 시작하면 보내요`, while the agent refused the line's first one): the operator's bubble, its files
 * with it, with `대기 중` and `✕`, which takes it out before it goes (it is not in the conversation until its turn starts).
 */
export function DeskQueuedBubble({ message, held = false, onRemove, onMention }: { message: DeskQueuedMessage; held?: boolean; onRemove: () => void; onMention?: (mention: DeskMention) => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex min-w-0 flex-col items-end gap-xxs" data-desk-queued={message.id}>
      <div className="opacity-(--opacity-dimmed)"><DeskUserMessage text={message.text} mentions={message.mentions} attachments={message.attachments} onMention={onMention} /></div>
      <span className="flex items-center gap-xxs text-micro text-muted-foreground">
        {t(held ? "desk.queued.held" : "desk.queued.waiting")}
        <Hint label={t("desk.queued.remove")}>
          <Button variant="ghost" size="icon-xs" aria-label={t("desk.queued.remove")} data-desk-queued-remove="true" onClick={onRemove}><XIcon /></Button>
        </Hint>
      </span>
    </div>
  );
}

/**
 * A mission a Desk answer names (`@mission(<id> <title>)`): its title where
 * the answer names it, in the title the app holds while it holds the mission;
 * its chip, two lines, stands in a row behind the paragraph (`DeskReply`).
 */
function DeskMissionMention({ mention }: { mention: DeskMissionMention }) {
  const title = useShellStore((s) => s.rest?.missions?.items.find((one) => one.id === mention.id)?.title) ?? mention.title;
  return (
    <span className="inline-flex max-w-full min-w-0 items-center align-middle" data-desk-mission-mention={mention.id}>
      <span className="truncate font-semibold text-foreground">{title}</span>
    </span>
  );
}

/** A reply's defaults, one array each, so a live answer that names nothing keeps the same chips while its words stream. */
const NO_REPLY_MENTIONS: readonly DeskMention[] = [];
const NO_REPLY_MISSIONS: readonly DeskMissionMention[] = [];

/**
 * Desk's answer as plain Markdown (D-75), where a session it names is that
 * session's chip and a mission its title, with its mission chip in a row behind the paragraph, each pressed to
 * open it; the core reads both from the finished answer, so a `live` one
 * streams its text alone; a stopped answer keeps what it said and says it
 * stopped (B7). Given `copy`, it offers `복사` of its original words, each
 * session and mission said by its name, on hover and on every code block
 * (Doop E2).
 */
export function DeskReply({ text, stopped = false, live = false, mentions = NO_REPLY_MENTIONS, missions = NO_REPLY_MISSIONS, onMention, onMission, onFile, copy }: { text: string; stopped?: boolean; live?: boolean; mentions?: readonly DeskMention[]; missions?: readonly DeskMissionMention[]; onMention?: (mention: DeskMention) => void; onMission?: (missionId: string) => void; /** A file path the answer names, pressed (feedback 2 frame M [2]); without it paths stay words. */ onFile?: (file: FoundAnswerPath) => void; copy?: CopyText }) {
  const { t } = useInterfaceTranslation();
  // Each chip is drawn in a root of its own inside the Markdown, so the screen's chip source is bridged to it.
  const bridge = useSessionChipBridge();
  // Desk runs on the core's own machine, in a folder of its own that is no project: its relative paths name nothing to chip.
  const here = useShellStore((s) => localDeviceId(s.rest));
  const place = useMemo<AnswerPlace | null>(() => (onFile ? { device: here, cwd: null } : null), [onFile, here]);
  const files = useFilePathChips(text, place, live, onFile ?? noFile);
  // A mission the app does not hold keeps the answer's title with no chip. The selector returns the ids of this answer's held
  // missions as one string, so a mission changing, or one this answer does not name, never redraws the answer (its text selection stays).
  const heldIds = useShellStore((s) => missions.filter((mission) => s.rest?.missions?.items.some((one) => one.id === mission.id)).map((mission) => mission.id).join(" "));
  const chips = useMemo<MarkdownChip[]>(() => [
    ...mentions.map((mention) => ({
      token: markedMention(mention),
      node: bridge(<SessionChip session={{ name: mention.name, mark: mention.mark ?? null, agent_kind: mention.agent_kind ?? null, pane_id: mention.id }} onPress={onMention ? () => onMention(mention) : undefined} />),
    })),
    ...missions.map((mission) => ({ token: markedMission(mission), node: bridge(<DeskMissionMention mention={mission} />), after: onMission && heldIds.split(" ").includes(mission.id) ? bridge(<MissionChipOf id={mission.id} onOpen={onMission} />) : undefined })),
    ...files,
  ], [mentions, missions, heldIds, onMention, onMission, bridge, files]);
  const words = <MarkdownText text={text} size="chat" chips={chips} copyCode={copy} />;
  return (
    <div className="flex min-w-0 flex-col gap-xxs" data-desk-item="reply" data-desk-reply={live ? "live" : stopped ? "stopped" : "done"}>
      {copy ? <CopyableMessage text={missions.reduce((said, mission) => said.split(markedMission(mission)).join(mission.title), namedText(text, mentions))} copy={copy}>{words}</CopyableMessage> : words}
      {stopped ? <span className="text-micro text-muted-foreground">{t("desk.reply.stopped")}</span> : null}
    </div>
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
  host_stopped: "desk.notice.host_stopped",
  resume_failed: "desk.notice.resume_failed",
  turn_failed: "desk.notice.turn_failed",
  attachments_gone: "desk.notice.attachments_gone",
};

export type DeskNoticeAction = "retry" | "restart" | "reinstall";

/** The one action that answers a notice, from its kind. */
const NOTICE_ACTION: Partial<Record<DeskNoticeKind, DeskNoticeAction>> = {
  install_failed: "reinstall",
  host_stopped: "restart",
  login_needed: "restart",
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
 * (D-75): what happened and the one action that answers it, as words, a
 * failure in the failed colour. A sign-in notice carries the command to run,
 * copyable; the words the operator sent stay in the composer. A failed turn
 * names the agent's own reason in plain words when it gave one and the reply
 * above did not already say it (`reason`); once a newer turn ran or a message waits
 * (`settled`) its `다시 시도` is gone, since the answer to it was given; the
 * notice that stopped a held line (`restartable`) offers `다시 시작`, which frees it.
 */
export function DeskNotice({ item, reason = failureReason(item.detail, null), settled = false, restartable = false, onAction, onCopy }: { item: Extract<DeskItem, { kind: "notice" }>; reason?: string | null; settled?: boolean; restartable?: boolean; onAction: (action: DeskNoticeAction) => void; onCopy: CopyText }) {
  const { t } = useInterfaceTranslation();
  const turn = item.notice === "turn_failed";
  // The failed colour is the failed turn's alone, as Doop's DeskNotice draws it; a stopped host is neutral words, as its `stopped`.
  const tone = turn ? "text-destructive" : item.notice === "install_failed" ? "text-warning" : item.notice === "preparing" ? "animate-pulse text-muted-foreground" : "text-subtle-foreground";
  const failed = turn || item.notice === "install_failed";
  // The notice that stopped a held line always offers the restart that frees it, whatever its kind.
  const action = settled ? undefined : NOTICE_ACTION[item.notice] ?? (restartable ? "restart" : undefined);
  const key = turn && reason ? "desk.notice.turn_failed_why" : NOTICE[item.notice];
  const words = t(key, { agent: item.agent ? agentProductName(item.agent) : t("desk.name"), detail: turn ? (reason ?? "") : (item.detail ?? "") });
  return (
    <p className="flex min-w-0 flex-wrap items-center gap-x-sm gap-y-xxs text-caption" role={failed ? "alert" : "status"} data-desk-item="notice" data-desk-notice={item.notice}>
      <span className={cn("min-w-0", tone)}>{words}</span>
      {item.notice === "login_needed" && item.detail ? (
        <span className="inline-flex min-w-0 items-center gap-xxs rounded-xs bg-secondary px-xs font-mono text-micro text-foreground">
          <TerminalIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
          <span className="truncate">{item.detail}</span>
          <Hint label={t("desk.action.copy")}>
            <Button variant="ghost" size="icon-xs" aria-label={t("desk.action.copy")} onClick={() => void onCopy(item.detail!, "desk sign-in command")}><CopyIcon /></Button>
          </Hint>
        </span>
      ) : null}
      {action ? <Button variant="text-link" size="inline" className="text-caption" data-desk-notice-action={action} onClick={() => onAction(action)}>{t(item.notice === "login_needed" && action === "restart" ? "desk.action.signedIn" : ACTION_WORDS[action])}</Button> : null}
    </p>
  );
}

/** The answered line's mark: what the card asked to do. */
const APPROVAL_ICON: Record<DeskApproval["card"], typeof SquareTerminalIcon> = {
  command: SquareTerminalIcon,
  edit: FilePenIcon,
  read: FileSearchIcon,
  other: ShieldIcon,
};

const APPROVAL_CARD: Record<DeskApproval["card"], MessageKey> = {
  command: "desk.approval.card.command",
  edit: "desk.approval.card.edit",
  read: "desk.approval.card.read",
  other: "desk.approval.card.other",
};

/**
 * The command on a card, folded to two lines when it is longer, with `전체
 * 보기` that shows it whole and `접기` that folds it again.
 */
function ApprovalCommand({ command }: { command: string }) {
  const { t } = useInterfaceTranslation();
  const block = useRef<HTMLSpanElement>(null);
  const [whole, setWhole] = useState(false);
  const [long, setLong] = useState(false);
  useLayoutEffect(() => {
    const element = block.current;
    if (!element || whole) return;
    const measure = () => setLong(element.scrollHeight > element.clientHeight);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [command, whole]);
  return (
    <div className="flex min-w-0 flex-col items-start gap-xxs">
      {/* The clamp sits inside the padding, so a folded block ends on its second line. */}
      <pre className="w-full min-w-0 whitespace-pre-wrap break-all rounded-sm bg-secondary px-sm py-xs font-mono text-caption text-foreground" data-desk-approval-command={whole ? "whole" : long ? "folded" : "short"}>
        <span ref={block} className={whole ? "block" : "line-clamp-2"}>{command}</span>
      </pre>
      {long || whole ? (
        <Button variant="text" size="inline" className="text-micro text-muted-foreground" aria-expanded={whole} data-desk-approval-command-toggle="true" onClick={() => setWhole(!whole)}>
          {t(whole ? "desk.approval.collapse" : "desk.approval.expand")}
          {whole ? <ChevronUpIcon aria-hidden="true" /> : <ChevronDownIcon aria-hidden="true" />}
        </Button>
      ) : null}
    </div>
  );
}

/** The answered line's words: what was pressed, by the kind of the agent's option it was. */
function answeredWord(item: DeskApproval): MessageKey {
  switch (item.state) {
    case "allowed":
      return item.chosen === "allow_once" ? "desk.approval.allowed" : item.chosen === "allow_always" ? "desk.approval.allowedAlways" : "desk.approval.allowedPlain";
    case "rejected":
      return item.refusal ? "desk.approval.rejectedWithLine" : "desk.approval.rejected";
    case "closed":
    case "waiting":
      return "desk.approval.closed";
  }
}

/** A card's buttons by what pressing them does, not by the word they read (an agent with two options of a kind has no word on either): a refusal as words, an allow-once option the card's primary, any other secondary. */
function buttonVariant(button: ApprovalButton): "text" | "secondary" | "ask" {
  if (refuses(button)) return "text";
  return button.option?.kind === "allow_once" ? "ask" : "secondary";
}

const BUTTON_WORD: Record<ApprovalWord, MessageKey> = {
  reject: "desk.approval.reject",
  allow_always: "desk.approval.allowAlways",
  allow_once: "desk.approval.allow",
};

/**
 * What Desk's agent asks to run, waiting for the operator (D-92, frame A2 가):
 * the card on the ask surface (`border-warning/50 bg-warning/10`) without an
 * agent mark, since it is Desk's own, its kind (`셸 명령`) first in the ask
 * colour and then `Desk가 이 일을 하려고 해요`. It comes
 * under `에이전트대로` for each request the agent makes, and under `다 허용`
 * only for one with no allow-once answer. The command on one grey block,
 * folded to two lines when longer, the folder, Desk's reason when it gave
 * one, and the agent's own answers (`approvalButtons`), `거절…` as words, `항상 허용`
 * secondary and `한 번 허용` the one primary in the ask colour (`buttonVariant`): `거절…` opens one line whose words go to Desk with the
 * refusal, `항상 허용` and `한 번 허용` go to the agent as its own options, so
 * what the agent keeps allowed is the agent's, as in the terminal. Every
 * button names the agent's own option on hover. Once answered it folds to one
 * dim line: how it was answered and the command's start.
 */
export function DeskApprovalCard({ item, onAnswer, defaultRefusal }: { item: DeskApproval; /** The agent's option pressed, or null for Hide's own `거절…`, with `거절…`'s line for a refusal. */ onAnswer: (option: string | null, text?: string) => void; /** Opens with `거절…`'s line already open and holding these words, for the gallery. */ defaultRefusal?: string }) {
  const { t } = useInterfaceTranslation();
  const buttons = approvalButtons(item.options);
  const firstReject = buttons.find(refuses) ?? null;
  const [refusing, setRefusing] = useState<ApprovalButton | null>(defaultRefusal !== undefined ? firstReject : null);
  const [line, setLine] = useState(defaultRefusal ?? "");
  if (item.state !== "waiting") {
    const Icon = APPROVAL_ICON[item.card];
    return (
      <p className="flex min-w-0 items-center gap-xs text-micro text-muted-foreground" title={item.command} data-desk-item="approval" data-desk-approval={item.state} data-desk-approval-answered={item.state} data-desk-approval-chosen={item.chosen ?? undefined}>
        <Icon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" data-desk-approval-card={item.card} />
        {/* An option Hide has no word for reads as the agent named it. */}
        <span className="shrink-0">{item.chosen === "other" && item.answer ? item.answer : t(answeredWord(item))}</span>
        <span aria-hidden="true">·</span>
        <span className="min-w-0 truncate font-mono">{item.command}</span>
      </p>
    );
  }
  const refuse = ({ option }: ApprovalButton) => onAnswer(option?.id ?? null, line.trim() || undefined);
  return (
    <section className="flex min-w-0 flex-col gap-xs rounded-lg border border-warning/50 bg-warning/10 px-md py-sm" aria-label={t(APPROVAL_CARD[item.card])} data-desk-item="approval" data-desk-approval={item.state}>
      <p className="flex min-w-0 items-baseline gap-sm">
        <span className="shrink-0 text-micro font-semibold text-warning" data-desk-approval-kind-label={item.card}>{t(APPROVAL_CARD[item.card])}</span>
        <span className="min-w-0 truncate font-medium text-foreground">{t("desk.approval.title")}</span>
      </p>
      <div className="flex min-w-0 flex-col gap-xs">
      <ApprovalCommand command={item.command} />
      {item.folder ? (
        <p className="flex min-w-0 items-baseline gap-xs text-micro text-muted-foreground">
          <span className="shrink-0">{t("desk.approval.folder")}</span>
          <span className="min-w-0 truncate font-mono text-subtle-foreground" title={item.folder}>{item.folder}</span>
        </p>
      ) : null}
      {item.reason ? <p className="min-w-0 whitespace-normal break-keep text-caption text-subtle-foreground [overflow-wrap:anywhere]">{item.reason}</p> : null}
      {refusing ? (
        <div className="flex min-w-0 flex-col gap-xs pt-xxs">
          <Input
            autoFocus
            value={line}
            maxLength={DESK_REFUSAL_MAX}
            placeholder={t("desk.approval.refusalPlaceholder")}
            aria-label={t("desk.approval.refusalLabel")}
            data-desk-approval-refusal="true"
            onChange={(event) => setLine(event.target.value)}
            onKeyDown={(event) => {
              if (composingKey(event)) return;
              if (event.key === "Enter") { event.preventDefault(); refuse(refusing); }
              else if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); setRefusing(null); setLine(""); }
            }}
          />
          <div className="flex flex-wrap items-center justify-end gap-xs">
            <Button variant="secondary" size="sm" data-desk-approval-answer="reject_plain" onClick={() => onAnswer(refusing.option?.id ?? null)}>{t("desk.approval.rejectPlain")}</Button>
            <Button variant="secondary" size="sm" data-desk-approval-answer="reject_send" onClick={() => refuse(refusing)}>{t("desk.approval.rejectSend")}<CornerDownLeftIcon aria-hidden="true" /></Button>
          </div>
        </div>
      ) : buttons.length > 0 ? (
        <div className="flex flex-wrap items-center justify-end gap-xs pt-xxs">
          {buttons.map((button) => {
            const { option, word } = button;
            const label = word ? t(BUTTON_WORD[word]) : (option?.name ?? "");
            // The button keeps its own word as its name; the hint adds the agent's own name for the option when the word is Hide's.
            return (
              <Hint key={option?.id ?? "reject"} label={option?.name ?? label} active={word !== null && option !== null}>
                <Button variant={buttonVariant(button)} size="sm" aria-label={label} data-desk-approval-option={option?.id} data-desk-approval-kind={option?.kind ?? "reject"} onClick={() => (refuses(button) ? setRefusing(button) : onAnswer(option?.id ?? null))}>
                  {label}
                </Button>
              </Hint>
            );
          })}
        </div>
      ) : null}
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
  /** The agent's option pressed on card `id`, with `거절…`'s line for a refusal. */
  onApproval: (id: number, option: string | null, text?: string) => void;
  onQueuedRemove: (id: number) => void;
  onNoticeAction: (action: DeskNoticeAction, notice: Extract<DeskItem, { kind: "notice" }>) => void;
  /** Puts words on the clipboard: a sign-in command, a message, a code block. */
  onCopy: CopyText;
  onMention: (mention: DeskMention) => void;
  onHanded: (paneId: string) => void;
  /** A file path an answer names, pressed: a project's file opens in the tab panel, an outside one shows in the file manager. */
  onFile?: (file: FoundAnswerPath) => void;
};

const noFile = () => undefined;

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

/** Whether the item after `index` is the notice that a failed turn did not finish, which says the reply stopped. */
function failsAfter(items: readonly DeskItem[], index: number): boolean {
  const next = items[index + 1];
  return next?.kind === "notice" && next.notice === "turn_failed";
}

/** The reply a notice follows directly, which a failed turn's notice does not repeat. */
function replyBefore(items: readonly DeskItem[], index: number): string | null {
  const before = items[index - 1];
  return before?.kind === "reply" ? before.text : null;
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
 * each job Desk started. `live` is the answer still arriving; `working`,
 * when given, is the working line at the foot until the turn ends.
 */
export function DeskChat({ items, jobs, agents, queued = [], held = false, live = null, working = null, selectedJob = null, selectedOutput = null, ...actions }: {
  items: readonly DeskItem[];
  jobs: Readonly<Record<string, DeskJob>>;
  /** The agents, whose choices name a model line's model and effort. */
  agents: readonly DeskAgent[];
  /** Messages waiting for the turn before them, after everything else. */
  queued?: readonly DeskQueuedMessage[];
  /** The line is stopped on its first message until a send or a restart. */
  held?: boolean;
  /** The answer streaming now, after the last item. */
  live?: string | null;
  /** The turn under way, which the working line says (feedback 1 c3): what it does now and when it started; none once it ends or while it waits on the operator. */
  working?: { phase: WorkingPhase; since: number | null } | null;
  selectedJob?: string | null;
  selectedOutput?: string | null;
} & DeskChatActions) {
  const { t } = useInterfaceTranslation();
  // A model line holds the choice's id; its words are the choice's label, or the id an agent no longer offers.
  const choice = (id: string | null, of: "models" | "efforts") => id === null ? t("desk.bar.default") : (agents.flatMap((agent) => agent[of]).find((one) => one.id === id)?.label ?? id);
  return (
    <ol className="mx-auto flex w-full max-w-(--size-editor-measure) min-w-0 flex-col gap-md px-lg py-lg text-subhead" aria-label={t("desk.name")} data-desk-chat={items.length}>
      {items.map((item, index) => (
        <li key={item.id} className="flex min-w-0 flex-col">
          {item.kind === "user" ? <DeskUserMessage text={item.text} mentions={item.mentions} attachments={item.attachments} onMention={actions.onMention} copy={actions.onCopy} />
            : item.kind === "reply" ? <div className="flex min-w-0 flex-col gap-xs"><DeskReplySteps thoughtMs={item.thought_ms} steps={item.steps} /><DeskReply text={item.text} stopped={item.state === "stopped" && !failsAfter(items, index)} mentions={replyMentions(item)} missions={item.missions} onMention={actions.onMention} onMission={actions.onMission} onFile={actions.onFile} copy={actions.onCopy} /></div>
            : item.kind === "switch" ? <MarkerLine icon={<ArrowLeftRightIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />} data-desk-item="switch">{item.from === null ? t("desk.switch.start", { to: agentProductName(item.to) }) : t(SWITCH_WORDS[item.carried], { from: agentProductName(item.from), to: agentProductName(item.to) })}</MarkerLine>
            : item.kind === "model" ? <MarkerLine icon={<SlidersHorizontalIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />} data-desk-item="model">{t("desk.model", { model: choice(item.model, "models"), effort: choice(item.effort, "efforts") })}</MarkerLine>
            : item.kind === "approval" ? <DeskApprovalCard item={item} onAnswer={(option, text) => actions.onApproval(item.id, option, text)} />
            : item.kind === "notice" ? <DeskNotice item={item} reason={failureReason(item.detail, replyBefore(items, index))} settled={(queued.length > 0 && item.notice === "turn_failed") || items.slice(index + 1).some((later) => later.kind === "user")} restartable={held && !items.slice(index + 1).some((later) => later.kind === "notice")} onAction={(action) => actions.onNoticeAction(action, item)} onCopy={actions.onCopy} />
            : item.kind === "handed" ? <DeskHandedLine session={handedSession(item)} handed={item.letter_kind === "request"} line={item.line} read={item.read} onOpen={() => actions.onHanded(item.pane_id)} />
            : <DeskJobCard job={jobOf(jobs, item.job_id)} selected={selectedJob === item.job_id} selectedOutput={selectedOutput} {...actions} />}
        </li>
      ))}
      {live !== null ? <li className="flex min-w-0 flex-col"><DeskReply text={live} live /></li> : null}
      {working ? <li data-desk-item="working"><WorkingLine phase={working.phase} since={working.since} /></li> : null}
      {queued.map((message) => <li key={`queued-${message.id}`} className="flex min-w-0 flex-col"><DeskQueuedBubble message={message} held={held} onRemove={() => actions.onQueuedRemove(message.id)} onMention={actions.onMention} /></li>)}
    </ol>
  );
}

/**
 * The empty Desk (D-68, B2, D-75): what Desk does in a line, three examples
 * as plain rows a press sends, and while threads run their real marks and
 * counts with the way to them. ◆ stays in the sidebar (D-38).
 */
export function DeskEmpty({ working, asking, onExample }: { working: number; asking: number; onExample: (text: string) => void }) {
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
        </p>
      ) : null}
    </div>
  );
}
