import { ArrowLeftRightIcon, CircleAlertIcon, CopyIcon, LoaderCircleIcon, SlidersHorizontalIcon, TerminalIcon } from "lucide-react";
import type { ReactNode } from "react";
import type { DeskItem, DeskJob, DeskMention } from "../../desk";
import { useInterfaceTranslation } from "../../i18n/client";
import type { MessageKey } from "../../i18n/catalogs";
import { cn } from "../../lib/utils";
import { MentionChip } from "../mention-chip";
import { Button } from "../ui/button";
import { Hint } from "../ui/tooltip";
import { DeskHandedCard, DeskJobCard, type DeskJobCardActions } from "./desk-job-card";

/**
 * Words with their `@name`s drawn as chips (B18, B26): a session and a
 * project read apart by colour; a chip with a press opens what it names.
 */
export function MentionText({ text, mentions, onMention }: { text: string; mentions: readonly DeskMention[]; onMention?: (mention: DeskMention) => void }) {
  if (mentions.length === 0) return <>{text}</>;
  const names = [...mentions].sort((a, b) => b.name.length - a.name.length);
  const parts: ReactNode[] = [];
  let rest = text;
  let key = 0;
  while (rest.length > 0) {
    const at = rest.indexOf("@");
    if (at < 0) break;
    const mention = names.find((one) => rest.startsWith(`@${one.name}`, at));
    if (!mention) {
      parts.push(rest.slice(0, at + 1));
      rest = rest.slice(at + 1);
      continue;
    }
    if (at > 0) parts.push(rest.slice(0, at));
    parts.push(onMention
      ? <MentionChip key={key++} kind={mention.kind} name={mention.name} label={mention.name} onPress={() => onMention(mention)} />
      : <MentionChip key={key++} kind={mention.kind} name={mention.name} />);
    rest = rest.slice(at + 1 + mention.name.length);
  }
  parts.push(rest);
  return <>{parts}</>;
}

/** The operator's own words, at the right (B7). */
export function DeskUserMessage({ text, mentions, onMention }: { text: string; mentions: readonly DeskMention[]; onMention?: (mention: DeskMention) => void }) {
  return (
    <p className="max-w-(--size-pr-popover) self-end whitespace-pre-wrap break-keep rounded-lg bg-secondary px-md py-xs text-body text-foreground [overflow-wrap:anywhere]" data-desk-item="user">
      <MentionText text={text} mentions={mentions} onMention={onMention} />
    </p>
  );
}

/** Desk's answer, streaming while `live`; a stopped answer keeps what it said and says it stopped (B7). */
export function DeskReply({ text, stopped = false, live = false }: { text: string; stopped?: boolean; live?: boolean }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex min-w-0 flex-col gap-xxs" data-desk-item="reply" data-desk-reply={live ? "live" : stopped ? "stopped" : "done"}>
      <p className="whitespace-pre-wrap break-keep text-body text-foreground [overflow-wrap:anywhere]">{text}</p>
      {stopped ? <span className="text-micro text-muted-foreground">{t("desk.reply.stopped")}</span> : null}
    </div>
  );
}

/** `생각 중` from the moment a message is sent until the answer starts (B7). */
export function DeskThinking() {
  const { t } = useInterfaceTranslation();
  return (
    <p role="status" className="flex items-center gap-xs text-caption text-muted-foreground" data-desk-item="thinking">
      <LoaderCircleIcon aria-hidden="true" className="size-(--size-icon-sm) animate-spin" />
      {t("desk.thinking")}
    </p>
  );
}

/** A small line between messages: where the agent or its model changed (B8, B9). */
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
};

const NOTICE_ACTION: Record<"retry" | "restart" | "reinstall", MessageKey> = {
  retry: "desk.action.retry",
  restart: "desk.action.restart",
  reinstall: "desk.action.reinstall",
};

/**
 * A one-line notice at the chat's end (B5, B6, B11, B13, B15): what happened
 * and the one action that answers it. A sign-in notice carries the command
 * to run, copyable; the words the operator sent stay in the composer.
 */
export function DeskNotice({ item, onAction, onCopy }: { item: Extract<DeskItem, { kind: "notice" }>; onAction: (action: "retry" | "restart" | "reinstall") => void; onCopy: (text: string) => void }) {
  const { t } = useInterfaceTranslation();
  const failed = item.notice === "blocked" || item.notice === "host_stopped" || item.notice === "install_failed";
  const words = t(NOTICE[item.notice], { agent: item.agent ?? "", detail: item.detail ?? "" });
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-x-sm gap-y-xxs text-caption" role={failed ? "alert" : "status"} data-desk-item="notice" data-desk-notice={item.notice}>
      {item.notice === "preparing" ? <LoaderCircleIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 animate-spin text-muted-foreground" /> : <CircleAlertIcon aria-hidden="true" className={cn("size-(--size-icon-sm) shrink-0", failed ? "text-destructive" : "text-warning")} />}
      <span className={cn("min-w-0", failed ? "text-destructive" : "text-subtle-foreground")}>{words}</span>
      {item.notice === "login_needed" && item.detail ? (
        <span className="inline-flex min-w-0 items-center gap-xxs rounded-xs bg-secondary px-xs font-mono text-micro text-foreground">
          <TerminalIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
          <span className="truncate">{item.detail}</span>
          <Hint label={t("desk.action.copy")}>
            <Button variant="ghost" size="icon-xs" aria-label={t("desk.action.copy")} onClick={() => onCopy(item.detail!)}><CopyIcon /></Button>
          </Hint>
        </span>
      ) : null}
      {item.action ? <Button variant="text-link" size="inline" className="text-caption" data-desk-notice-action={item.action} onClick={() => onAction(item.action!)}>{t(NOTICE_ACTION[item.action])}</Button> : null}
    </div>
  );
}

/**
 * Desk asking before it runs a command or changes code (D-52, B14): the
 * command whole, the folder, Desk's one-line reason and two answers. An
 * answered card keeps the command and says how it was answered.
 */
export function DeskApprovalCard({ item, onAnswer }: { item: Extract<DeskItem, { kind: "approval" }>; onAnswer: (allow: boolean) => void }) {
  const { t } = useInterfaceTranslation();
  const waiting = item.state === "waiting";
  const answered = item.state === "allowed" ? t("desk.approval.allowed") : item.state === "rejected" ? t("desk.approval.rejected") : item.state === "closed" ? t("desk.approval.closed") : null;
  return (
    <section className={cn("flex min-w-0 flex-col gap-xs rounded-md border bg-card px-md py-sm", waiting ? "border-warning" : "border-border")} aria-label={item.title} data-desk-item="approval" data-desk-approval={item.state}>
      <p className="flex min-w-0 items-center gap-xs text-caption">
        <span className={cn("font-medium", waiting ? "text-foreground" : item.state === "allowed" ? "text-success" : "text-muted-foreground")} data-desk-approval-answered={waiting ? undefined : item.state}>{waiting ? t("desk.approval.title") : answered}</span>
        <span className="min-w-0 truncate text-muted-foreground">{item.title}</span>
      </p>
      <pre className={cn("min-w-0 whitespace-pre-wrap break-all rounded-sm bg-secondary px-sm py-xs font-mono text-caption text-foreground", !waiting && "opacity-(--opacity-dimmed)")} data-desk-approval-command="true">{item.command}</pre>
      <p className="flex min-w-0 items-baseline gap-xs text-micro text-muted-foreground">
        <span className="shrink-0">{t("desk.approval.folder")}</span>
        <span className="min-w-0 truncate font-mono text-subtle-foreground" title={item.folder}>{item.folder}</span>
      </p>
      <p className="min-w-0 whitespace-normal break-keep text-caption text-subtle-foreground [overflow-wrap:anywhere]">{item.reason}</p>
      {waiting ? (
        <div className="flex items-center justify-end gap-xs pt-xxs">
          <Button variant="outline" size="sm" data-desk-approval-answer="reject" onClick={() => onAnswer(false)}>{t("desk.approval.reject")}</Button>
          <Button size="sm" data-desk-approval-answer="allow" onClick={() => onAnswer(true)}>{t("desk.approval.allow")}</Button>
        </div>
      ) : null}
    </section>
  );
}

export type DeskChatActions = DeskJobCardActions & {
  onApproval: (id: string, allow: boolean) => void;
  onNoticeAction: (action: "retry" | "restart" | "reinstall") => void;
  onCopy: (text: string) => void;
  onMention: (mention: DeskMention) => void;
  onHanded: (agentId: string) => void;
};

/**
 * Desk's conversation (D-08, B7-B9, B13-B16): the operator's words, Desk's
 * answers, a job's card where Desk started it, a handed task's notice,
 * approval cards, the switch and model lines and the notices, in order;
 * `thinking` and `live` draw the answer under way.
 */
export function DeskChat({ items, jobs, live = null, thinking = false, selectedJob = null, selectedOutput = null, ...actions }: {
  items: readonly DeskItem[];
  jobs: Readonly<Record<string, DeskJob>>;
  /** The answer streaming now, after the last item. */
  live?: string | null;
  thinking?: boolean;
  selectedJob?: string | null;
  selectedOutput?: string | null;
} & DeskChatActions) {
  const { t } = useInterfaceTranslation();
  return (
    <ol className="mx-auto flex w-full max-w-(--size-editor-measure) min-w-0 flex-col gap-md px-lg py-lg" aria-label={t("desk.name")} data-desk-chat={items.length}>
      {items.map((item) => (
        <li key={item.id} className="flex min-w-0 flex-col">
          {item.kind === "user" ? <DeskUserMessage text={item.text} mentions={item.mentions} onMention={actions.onMention} />
            : item.kind === "reply" ? <DeskReply text={item.text} stopped={item.state === "stopped"} />
            : item.kind === "switch" ? <MarkerLine icon={<ArrowLeftRightIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />} data-desk-item="switch">{t(item.carried ? "desk.switch.carried" : "desk.switch.trimmed", { from: item.from, to: item.to })}</MarkerLine>
            : item.kind === "model" ? <MarkerLine icon={<SlidersHorizontalIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />} data-desk-item="model">{t("desk.model", { model: item.model, effort: item.effort })}</MarkerLine>
            : item.kind === "approval" ? <DeskApprovalCard item={item} onAnswer={(allow) => actions.onApproval(item.id, allow)} />
            : item.kind === "notice" ? <DeskNotice item={item} onAction={actions.onNoticeAction} onCopy={actions.onCopy} />
            : item.kind === "handed" ? <DeskHandedCard name={item.name} project={item.project} onOpen={() => actions.onHanded(item.agent_id)} />
            : jobs[item.job_id] ? <DeskJobCard job={jobs[item.job_id]!} selected={selectedJob === item.job_id} selectedOutput={selectedOutput} {...actions} /> : null}
        </li>
      ))}
      {live !== null ? <li className="flex min-w-0 flex-col"><DeskReply text={live} live /></li> : thinking ? <li><DeskThinking /></li> : null}
    </ol>
  );
}

/**
 * The empty Desk (D-68, B2): what Desk does in a line, three examples a press
 * sends, and while threads run their count and the way to them.
 */
export function DeskEmpty({ working, asking, onExample, onShowThreads }: { working: number; asking: number; onExample: (text: string) => void; onShowThreads: () => void }) {
  const { t } = useInterfaceTranslation();
  const examples = [t("desk.empty.example1"), t("desk.empty.example2"), t("desk.empty.example3")];
  return (
    <div className="mx-auto flex w-full max-w-(--size-pr-popover) min-w-0 flex-1 flex-col items-center justify-center gap-sm px-lg text-center" data-desk-empty="true">
      <h2 className="text-subhead font-semibold text-foreground">{t("desk.empty.title")}</h2>
      <p className="whitespace-normal break-keep text-caption text-muted-foreground">{t("desk.empty.body")}</p>
      <ul className="mt-sm flex w-full min-w-0 flex-col gap-xs">
        {examples.map((example) => (
          <li key={example}>
            <Button variant="outline" className="h-auto min-h-(--size-control) w-full justify-start whitespace-normal py-xs text-left font-normal text-subtle-foreground" data-desk-example="true" onClick={() => onExample(example)}>{example}</Button>
          </li>
        ))}
      </ul>
      {working + asking > 0 ? (
        <p className="mt-xs flex items-center gap-sm text-caption text-muted-foreground" data-desk-empty-live="true">
          <span>{t("desk.empty.live", { working, asking })}</span>
          <Button variant="text-link" size="inline" onClick={onShowThreads}>{t("desk.empty.showThreads")}</Button>
        </p>
      ) : null}
    </div>
  );
}
