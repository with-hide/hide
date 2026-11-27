import { ArrowUpIcon, ChevronDownIcon, HandIcon, LoaderCircleIcon, PaperclipIcon, ShieldIcon, SquareIcon, XIcon, type LucideIcon } from "lucide-react";
import { useRef, useState, type ClipboardEvent, type KeyboardEvent, type ReactNode, type Ref, type UIEvent } from "react";
import { MAX_BATCH_BYTES, MAX_FILE_BYTES, MAX_FILES } from "../../attachments";
import type { DeskAgent, DeskAgentId, DeskAgentWord, DeskChoice, DeskPermission, DeskSelection } from "../../desk";
import { agentPickable, deskAgentWord } from "../../desk";
import { traySendable, type ComposerTray, type SentAttachment, type TrayFile } from "../../deskComposer";
import { fileIcon } from "../../fileIcons";
import { useInterfaceTranslation } from "../../i18n/client";
import type { MessageKey } from "../../i18n/catalogs";
import { formatBytes } from "../../i18n/format";
import { requireInterfaceLanguage } from "../../i18n/locale";
import { cn } from "../../lib/utils";
import { AgentMark } from "../../AgentMark";
import { Badge } from "../ui/badge";
import { FileMark } from "../file-mark";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuSeparator, DropdownMenuTrigger } from "../ui/dropdown-menu";
import { Popover, PopoverAnchor, PopoverContent } from "../ui/popover";
import { Textarea } from "../ui/textarea";
import { Hint } from "../ui/tooltip";
import { composingKey } from "../../keys";
import { useNoticeFloor } from "../../noticeFloor";
import type { DeskDraft } from "../../ui";
import { DESK_MENU_DROPDOWN, DESK_MENU_GAP, DESK_MENU_HEADING, DESK_MENU_PANEL } from "./desk-menu";
import { useFileDropOver } from "./file-drop";
import { MentionInput, type MentionInputHandle } from "./mention-input";

const MIB = MAX_FILE_BYTES / (1024 * 1024);

/**
 * The words' height in both composers, Desk's and a session's: two lines at
 * rest, growing with the words up to about eight lines, then scrolling
 * inside, so the files above and the bar under them stay in view.
 */
const TEXT_HEIGHT = "max-h-(--size-desk-composer-text-max) min-h-[calc(var(--text-body)*1.5*2+var(--spacing-xxs)*2)]";
const BATCH_MIB = MAX_BATCH_BYTES / (1024 * 1024);

function useByteSize(): (bytes: number) => string {
  const { i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  return (bytes) => formatBytes(language, bytes);
}

/**
 * One attached file (D-27, B41, B42), in a composer's tray or over a sent
 * message: an image with a thumbnail is its picture, any other file its mark,
 * name and size. In the tray it has its `✕`, the band of its bytes read in
 * while it reads, and, past the cap, a red line and `20MB를 넘어요`.
 */
export function AttachmentTile({ name, size, preview, state = "ready", read = 0, sent = false, onRemove }: {
  name: string;
  size: number | null;
  preview: string | null;
  state?: TrayFile["state"];
  read?: number;
  sent?: boolean;
  onRemove?: () => void;
}) {
  const { t } = useInterfaceTranslation();
  const bytes = useByteSize();
  const refused = state === "too_large" || state === "over_batch" || state === "unreadable";
  const why = state === "too_large" ? t("desk.composer.tooLarge", { limit: MIB }) : state === "over_batch" ? t("desk.composer.overBatch", { count: MAX_FILES, limit: BATCH_MIB }) : state === "unreadable" ? t("desk.composer.unreadable") : null;
  const share = size && size > 0 ? Math.min(100, Math.round((read / size) * 100)) : 0;
  const band = state === "reading" ? (
    <span className="absolute inset-x-xs bottom-xxs h-xxs overflow-hidden rounded-full bg-border" role="progressbar" aria-label={name} aria-valuemin={0} aria-valuemax={100} aria-valuenow={share}>
      <span className="block h-full rounded-full bg-agent-working" style={{ width: `${share}%` }} />
    </span>
  ) : null;
  const face = preview ? (
    <span className={cn("relative block overflow-hidden rounded-md border border-border bg-secondary", sent ? "aspect-[3/2] w-(--size-desk-output-thumb)" : "size-(--size-desk-attach-thumb)")}>
      <img src={preview} alt="" className={cn("size-full object-cover", state === "reading" && "opacity-(--opacity-dimmed)")} />
      {band}
    </span>
  ) : (
    <span className={cn("relative flex h-(--size-desk-attach-thumb) max-w-(--size-tab-preferred) items-center gap-xs rounded-md border bg-secondary pr-sm pl-xs", refused ? "border-destructive" : "border-border", state === "reading" && "text-muted-foreground")}>
      <span className="flex size-(--size-form-control) shrink-0 items-center justify-center rounded-sm bg-background">
        <FileMark icon={fileIcon(name)} />
      </span>
      <span className="min-w-0 text-left">
        <span className="block truncate text-caption text-foreground">{name}</span>
        <span className={cn("block truncate text-micro", refused ? "text-destructive" : "text-muted-foreground")} data-desk-attachment-size="true">
          {why ?? (size === null ? "\u00a0" : bytes(size))}
        </span>
      </span>
      {band}
    </span>
  );
  return (
    <li className="relative shrink-0" data-desk-attachment={state} data-desk-attachment-kind={preview ? "image" : "file"}>
      <span role="img" aria-label={why ? `${name} · ${why}` : name}>{face}</span>
      {onRemove ? (
        <Button variant="secondary" size="icon-xs" className="absolute -top-xs -right-xs rounded-full border border-border" aria-label={t("desk.composer.removeAttachment", { name })} onClick={onRemove} data-desk-attachment-remove={name}><XIcon /></Button>
      ) : null}
    </li>
  );
}

/** The files a message carried, over its bubble at the right (B41). */
export function SentAttachments({ files }: { files: readonly SentAttachment[] }) {
  if (files.length === 0) return null;
  return (
    <ul className="flex max-w-(--size-pr-popover) flex-wrap justify-end gap-xs self-end" data-desk-sent-attachments={files.length}>
      {files.map((file, index) => <AttachmentTile key={`${index}-${file.name}`} name={file.name} size={file.size} preview={file.preview} sent />)}
    </ul>
  );
}

/**
 * The box the operator writes in, for Desk and for a session alike (B7,
 * B29, B41): the words, the attached files above them, 📎, and send, which
 * becomes stop while Desk answers. `menu` is the `@` menu, opened above the
 * box while `menuOpen`; `children` sits in the box's foot between 📎 and
 * send (Desk's bar). With `tray`, the box takes files: dropped anywhere on
 * the column it stands in (`FileDropZone`, whose drag lights the box's
 * border and nothing else), pasted into it, or picked with 📎; a send waits while one is still read
 * in or one is past the cap. With `draft`, the box is Desk's, where picked
 * mentions stand as chips (D-85) and `value` is the draft's text; without,
 * it is a plain text box.
 */
export function DeskComposer({ value, draft, inputRef, textRef, label, placeholder, tray, answering = false, stopHint, focused = false, held = false, notice = null, menu = null, menuOpen = false, menuEmpty = false, onChange, onCaret, onKeyDown, onSend, onHeld, onStop, onMenuOpenChange, children }: {
  value: string;
  /** Desk's draft: its mentions are drawn as chips in the box. */
  draft?: DeskDraft;
  inputRef?: Ref<MentionInputHandle>;
  /** The plain box, for a caller that places the caret after a pick. */
  textRef?: Ref<HTMLTextAreaElement>;
  label: string;
  placeholder: string;
  /** The files attached so far; without it the box takes none. */
  tray?: ComposerTray;
  /** Desk is answering, or a session's agent works: send becomes stop (B7, PRD desk-queue B10); ↵ still sends. */
  answering?: boolean;
  /** What stop's hint says it stops; Desk's answer when not given. */
  stopHint?: string;
  /** Drawn as the keyboard's place, as `답하기` leaves it (B29). */
  focused?: boolean;
  /** Send waits: the words have nowhere to go yet, or the last ones are still on their way. */
  held?: boolean;
  /** One line under the box: why the last words were not sent, or why none can be. */
  notice?: string | null;
  menu?: ReactNode;
  menuOpen?: boolean;
  /** The open menu holds nothing to choose, so ↵ is not its own and sends. */
  menuEmpty?: boolean;
  /** The words changed, and where the caret stands in them. */
  onChange: (value: string, caret: number) => void;
  /** The caret moved without the words changing. */
  onCaret?: (caret: number) => void;
  /** The box's keys before it takes them, such as the `@` menu's moves; one it prevents is not sent. */
  onKeyDown?: (event: KeyboardEvent<HTMLElement>) => void;
  onSend: () => void;
  /** ↵ with words or files while send waits: nothing is sent and the words stay. */
  onHeld?: () => void;
  onStop?: () => void;
  onMenuOpenChange?: (open: boolean) => void;
  children?: ReactNode;
}) {
  const { t } = useInterfaceTranslation();
  const picker = useRef<HTMLInputElement>(null);
  const files = tray?.files ?? [];
  const empty = value.trim() === "" && files.length === 0;
  const blocked = empty || held || (files.length > 0 && traySendable(files) === null);
  const keyDown = (event: KeyboardEvent<HTMLElement>) => {
    onKeyDown?.(event);
    // Enter sends and Shift+Enter starts a line, while Desk answers too (the message waits for its turn); a key that ends an IME composition only commits it, and the `@` menu takes its own Enter.
    if (event.defaultPrevented || event.key !== "Enter" || event.shiftKey || composingKey(event) || (menuOpen && !menuEmpty)) return;
    event.preventDefault();
    if (!blocked) onSend();
    else if (!empty && held) onHeld?.();
  };
  const over = useFileDropOver();
  // A paste that carries files attaches them; words paste as words.
  const paste = (event: ClipboardEvent<HTMLElement>) => {
    if (!tray || event.clipboardData.files.length === 0) return;
    event.preventDefault();
    tray.add([...event.clipboardData.files]);
  };
  // Words scrolled past the box's top fade out there, so the line cut under the files reads as more above.
  const [above, setAbove] = useState(false);
  const scrolled = (event: UIEvent<HTMLElement>) => setAbove(event.currentTarget.scrollTop > 0);
  const text = cn(TEXT_HEIGHT, above && "mask-t-from-75%");
  // A notice in the window's corner stands above the box, never over it.
  const box = useRef<HTMLDivElement>(null);
  useNoticeFloor(box);
  return (
    <div ref={box} className="flex min-w-0 flex-col gap-xs" data-desk-composer={answering ? "answering" : "idle"}>
      <Popover open={menuOpen} onOpenChange={onMenuOpenChange}>
        <PopoverAnchor asChild>
          <div
            className={cn("relative flex min-w-0 flex-col gap-xs rounded-xl border bg-background px-md py-sm", focused || menuOpen || over ? "border-ring ring-1 ring-ring" : "border-input")}
            data-desk-composer-frame={over ? "drop" : focused ? "focused" : "rest"}
            onPasteCapture={paste}
          >
            {files.length > 0 ? (
              // The files stand on a row of their own, clear of the words under them; the space above keeps each `✕` inside the frame.
              <ul className="flex min-w-0 flex-wrap gap-sm pt-xs pr-xs pb-sm" data-desk-attachments={files.length}>
                {files.map((file) => <AttachmentTile key={file.id} name={file.name} size={file.size} preview={file.preview} state={file.state} read={file.read} onRemove={() => tray?.remove(file.id)} />)}
              </ul>
            ) : null}
            {draft ? (
              <MentionInput ref={inputRef} draft={draft} label={label} placeholder={placeholder} autoFocus={focused} className={text} onScroll={scrolled} onChange={onChange} onCaret={onCaret} onKeyDown={keyDown} />
            ) : (
              <Textarea ref={textRef} bare rows={1} value={value} aria-label={label} onSelect={(event) => onCaret?.(event.currentTarget.selectionStart)} placeholder={placeholder} className={cn("field-sizing-content overflow-y-auto py-xxs", text)} onScroll={scrolled} autoFocus={focused} onChange={(event) => onChange(event.target.value, event.target.selectionStart)} data-desk-composer-input="true" onKeyDown={keyDown} />
            )}
            <div className="flex min-w-0 items-center gap-xs">
              {tray ? (
                <Hint label={t("desk.composer.attach")}>
                  <Button variant="ghost" size="icon-sm" className="-ml-xs text-muted-foreground" aria-label={t("desk.composer.attach")} onClick={() => picker.current?.click()} data-desk-composer-attach="true"><PaperclipIcon /></Button>
                </Hint>
              ) : null}
              <span className="flex min-w-0 flex-1 items-center justify-end">{children}</span>
              {answering ? (
                <Hint label={stopHint ?? t("desk.composer.stopHint")}>
                  <Button variant="secondary" size="icon-sm" className="rounded-full" aria-label={t("desk.composer.stop")} data-desk-composer-stop="true" onClick={onStop}><SquareIcon className="fill-current" /></Button>
                </Hint>
              ) : (
                <Hint label={t("desk.composer.send")} shortcut="↵">
                  <Button variant="secondary" size="icon-sm" className="rounded-full" aria-label={t("desk.composer.send")} disabled={blocked} data-desk-composer-send="true" onClick={onSend}><ArrowUpIcon /></Button>
                </Hint>
              )}
            </div>
          </div>
        </PopoverAnchor>
        {menu ? (
          <PopoverContent side="top" align="start" sideOffset={DESK_MENU_GAP} className={DESK_MENU_PANEL} onOpenAutoFocus={(event) => event.preventDefault()} data-desk-mention-popover="true">
            {menu}
          </PopoverContent>
        ) : null}
      </Popover>
      {notice ? <p role="status" className="px-xs text-caption text-warning" data-desk-composer-notice="true">{notice}</p> : null}
      {tray ? (
        <Input
          ref={picker}
          type="file"
          multiple
          hidden
          data-desk-composer-picker="true"
          onChange={(event) => {
            tray.add([...(event.target.files ?? [])]);
            event.target.value = "";
          }}
        />
      ) : null}
    </div>
  );
}

const AGENT_WORD: Record<DeskAgentWord, MessageKey> = {
  signed_in: "desk.agentState.signed_in",
  signed_out: "desk.agentState.signed_out",
  checking: "desk.agentState.checking",
  preparing: "desk.agentState.preparing",
  install_failed: "desk.agentState.install_failed",
  node_missing: "desk.agentState.node_missing",
  cli_missing: "desk.agentState.cli_missing",
  off: "desk.agentState.off",
};

/** Why an agent the bar lists cannot be picked, on hover. */
const UNPICKABLE_HINT: Partial<Record<DeskAgentWord, MessageKey>> = {
  off: "desk.agentState.offHint",
};

/** A word that asks the operator to act is in the warning color; one that is passing or settled is quiet. */
const QUIET_WORDS: ReadonlySet<DeskAgentWord> = new Set(["signed_in", "checking", "preparing", "off"]);

/** The word beside an agent: its sign-in or its state, with the spinner while its adapter downloads. */
function AgentWord({ word, ...data }: { word: DeskAgentWord } & Record<`data-${string}`, string>) {
  const { t } = useInterfaceTranslation();
  return (
    <span className={cn("flex shrink-0 items-center gap-xxs text-micro", QUIET_WORDS.has(word) ? "text-muted-foreground" : "text-warning")} {...data}>
      {word === "preparing" ? <LoaderCircleIcon aria-hidden="true" className="size-(--size-icon-sm) motion-safe:animate-spin" /> : null}
      {t(AGENT_WORD[word])}
    </span>
  );
}

/** One of the bar's three menus: its trigger names the value, and pressing it opens the choices. */
function BarMenu({ name, hint, value, open, onOpenChange, highlight = false, wide = false, disabled = false, children, ...data }: { name: string; /** The trigger's hint, its name by default. */ hint?: string; value: ReactNode; open?: boolean; onOpenChange?: (open: boolean) => void; highlight?: boolean; /** A menu whose choices carry a line under their name. */ wide?: boolean; disabled?: boolean; children: ReactNode } & Record<`data-${string}`, string>) {
  return (
    <DropdownMenu open={open} onOpenChange={onOpenChange} modal={false}>
      <Hint label={hint ?? name}>
        <DropdownMenuTrigger asChild disabled={disabled}>
          <Button variant="ghost" size="sm" className={cn("h-(--size-control-compact) gap-xxs border px-xs text-caption font-normal", highlight ? "border-hairline bg-secondary text-foreground" : "border-transparent text-subtle-foreground")} aria-label={name} {...data}>
            {value}
            <ChevronDownIcon className="size-(--size-icon-sm) text-muted-foreground" />
          </Button>
        </DropdownMenuTrigger>
      </Hint>
      <DropdownMenuContent align="end" side="top" sideOffset={DESK_MENU_GAP} className={cn(DESK_MENU_DROPDOWN, "max-h-(--radix-dropdown-menu-content-available-height)", wide ? "w-(--size-picker-popover) max-w-(--radix-dropdown-menu-content-available-width)" : "min-w-(--size-tab-preferred)")}>{children}</DropdownMenuContent>
    </DropdownMenu>
  );
}

/**
 * Under Desk's composer (D-66, B4, B5): the agent, its model and its effort,
 * each a menu. The agent menu lists the agents switched on in Settings ›
 * Agents (D-79), each with the sign-in Settings reads or its state (D-81);
 * one switched off since it was chosen is listed dim with its reason and
 * cannot be picked; switching keeps the conversation (B9). Before an agent is chosen
 * only the agent menu shows; before its first session the model and effort
 * read as the agent's default. `open` opens one menu, for review.
 */
export function DeskAgentBar({ agents, selection, open = null, onAgent, onModel, onEffort, onAgentSettings }: {
  agents: readonly DeskAgent[];
  selection: DeskSelection;
  open?: "agent" | "model" | "effort" | null;
  onAgent: (agent: DeskAgentId) => void;
  onModel: (model: string) => void;
  onEffort: (effort: string) => void;
  /** Opens Settings › Agents, where the list and the sign-ins come from. */
  onAgentSettings: () => void;
}) {
  const { t } = useInterfaceTranslation();
  const agent = selection.agent === null ? null : agents.find((one) => one.id === selection.agent);
  if (agent === undefined) throw new Error(`Desk selection names an unknown agent ${selection.agent}`);
  const choice = (choices: readonly DeskChoice[], id: string | null) => (id === null ? t("desk.bar.default") : (choices.find((one) => one.id === id)?.label ?? id));
  const chosenWord = agent ? deskAgentWord(agent) : null;
  const registered = agents.filter((one) => one.state !== "off").length;
  return (
    <div className="flex min-w-0 items-center justify-end gap-xxs" data-desk-agent-bar="true">
      {chosenWord && chosenWord !== "signed_in" ? <span className="mr-auto min-w-0 truncate"><AgentWord word={chosenWord} data-desk-agent-bar-state={chosenWord} /></span> : null}
      <BarMenu name={t("desk.bar.agent")} value={agent ? <><AgentMark kind={agent.id} />{agent.label}</> : t("desk.bar.agent")} open={open === "agent" ? true : undefined} highlight={open === "agent"} data-desk-bar="agent">
        <DropdownMenuLabel className={cn("flex items-center justify-between gap-sm", DESK_MENU_HEADING)}>
          <span>{t("desk.bar.agent")}</span>
          <span className="font-normal text-muted-foreground" data-desk-agent-count={registered}>{t("desk.bar.registered", { n: registered })}</span>
        </DropdownMenuLabel>
        <DropdownMenuRadioGroup value={selection.agent ?? undefined} onValueChange={(value) => onAgent(value as DeskAgentId)}>
          {agents.map((one) => {
            const pickable = agentPickable(one);
            const word = deskAgentWord(one);
            const item = (
              <DropdownMenuRadioItem key={one.id} value={one.id} disabled={!pickable} className="gap-sm" data-desk-agent={one.id} data-desk-agent-state={one.state} data-desk-agent-word={word ?? "none"}>
                <AgentMark kind={one.id} />
                <span className="flex-1 truncate">{one.label}</span>
                {word ? <AgentWord word={word} /> : null}
              </DropdownMenuRadioItem>
            );
            const hint = word ? UNPICKABLE_HINT[word] : undefined;
            return !pickable && hint ? <Hint key={one.id} label={t(hint)} side="left"><span>{item}</span></Hint> : item;
          })}
        </DropdownMenuRadioGroup>
        <DropdownMenuSeparator />
        <div className="flex items-center justify-between gap-sm">
          <DropdownMenuLabel className="font-normal text-muted-foreground">{t("desk.bar.carries")}</DropdownMenuLabel>
          <DropdownMenuItem onSelect={onAgentSettings} className="text-caption text-subtle-foreground" data-desk-agent-settings="true">{t("desk.bar.agentSettings")}</DropdownMenuItem>
        </div>
      </BarMenu>
      {agent ? (
        <>
          <BarMenu name={t("desk.bar.model")} value={choice(agent.models, selection.model)} open={open === "model" ? true : undefined} data-desk-bar="model">
            <DropdownMenuRadioGroup value={selection.model ?? undefined} onValueChange={onModel}>
              {agent.models.map((one) => <DropdownMenuRadioItem key={one.id} value={one.id}>{one.label}</DropdownMenuRadioItem>)}
            </DropdownMenuRadioGroup>
          </BarMenu>
          <BarMenu name={t("desk.bar.effort")} value={choice(agent.efforts, selection.effort)} open={open === "effort" ? true : undefined} data-desk-bar="effort">
            <DropdownMenuRadioGroup value={selection.effort ?? undefined} onValueChange={onEffort}>
              {agent.efforts.map((one) => <DropdownMenuRadioItem key={one.id} value={one.id}>{one.label}</DropdownMenuRadioItem>)}
            </DropdownMenuRadioGroup>
          </BarMenu>
        </>
      ) : null}
    </div>
  );
}

/** A slot of the bar that shows its value and cannot be changed here, with why on hover; no chevron, since it opens nothing. */
function BarValue({ name, hint, children, ...data }: { name: string; hint: string; children: ReactNode } & Record<`data-${string}`, string>) {
  return (
    <Hint label={hint}>
      <span tabIndex={0} aria-label={`${name} · ${hint}`} className="flex h-(--size-control-compact) min-w-0 items-center gap-xxs rounded-md px-xs text-caption text-subtle-foreground" {...data}>
        {children}
      </span>
    </Hint>
  );
}

/**
 * One slot of a session's bar: the value the session runs with (null for the
 * CLI's own) and its name, the choices a change can make, and, for a slot the
 * agent changes only in a menu of its own, why it cannot change here.
 */
export type SessionBarSlot = { value: string | null; label: string | null; choices: readonly DeskChoice[]; fixed: string | null };

/**
 * Under a session's input (mission-106): the bar Desk's composer has, with
 * the agent shown and fixed, since a session keeps the agent it runs. The
 * model and the effort are menus while the agent changes them by a typed
 * line in a running session (`[switch]` of its definition), each pick sent
 * as that line and shown as the session's once typed; `switchHint` says the
 * line a change types. A slot the agent changes only in its own menu shows
 * its value and says so on hover. `held` waits while a send is on its way.
 */
export function SessionAgentBar({ agent, model, effort, switchHint, held = false, open = null, onModel, onEffort }: {
  agent: { kind: string; label: string };
  model: SessionBarSlot;
  effort: SessionBarSlot;
  /** The command a change types, named in each menu's hint. */
  switchHint: { model: string | null; effort: string | null };
  held?: boolean;
  /** Opens one menu, for review. */
  open?: "model" | "effort" | null;
  onModel: (model: string) => void;
  onEffort: (effort: string) => void;
}) {
  const { t } = useInterfaceTranslation();
  const shown = (slot: SessionBarSlot) => slot.label ?? slot.value ?? t("desk.bar.default");
  const slot = (which: "model" | "effort", one: SessionBarSlot, pick: (value: string) => void) => {
    const name = t(which === "model" ? "desk.bar.model" : "desk.bar.effort");
    if (one.fixed !== null) return <BarValue name={name} hint={one.fixed} data-session-bar={which} data-session-bar-fixed="true">{shown(one)}</BarValue>;
    const command = switchHint[which];
    return (
      <BarMenu name={name} hint={command ? t("desk.bar.switchHint", { command }) : name} value={shown(one)} open={open === which ? true : undefined} disabled={held} data-session-bar={which}>
        <DropdownMenuRadioGroup value={one.value ?? undefined} onValueChange={(value) => value !== one.value && pick(value)}>
          {one.choices.map((choice) => <DropdownMenuRadioItem key={choice.id} value={choice.id} data-session-bar-choice={choice.id}>{choice.label}</DropdownMenuRadioItem>)}
        </DropdownMenuRadioGroup>
      </BarMenu>
    );
  };
  return (
    <div className="flex min-w-0 items-center justify-end gap-xxs" data-session-agent-bar={agent.kind}>
      <BarValue name={t("desk.bar.agent")} hint={t("desk.bar.agentFixed")} data-session-bar="agent"><AgentMark kind={agent.kind} />{agent.label}</BarValue>
      {slot("model", model, onModel)}
      {slot("effort", effort, onEffort)}
    </div>
  );
}

const PERMISSIONS: readonly DeskPermission[] = ["allow_all", "agent"];

const PERMISSION_NAME: Record<DeskPermission, MessageKey> = {
  allow_all: "desk.permission.allow_all",
  agent: "desk.permission.agent",
};

/** Each level's mark (feedback 1 frame P 가): a shield for `다 허용`, a raised hand for `에이전트대로`, which stops at each ask. */
const PERMISSION_ICON: Record<DeskPermission, LucideIcon> = {
  allow_all: ShieldIcon,
  agent: HandIcon,
};

/**
 * Under Desk's composer, before the agent (D-92, feedback 1 frame A2): how
 * Desk answers what its agent asks to run, the same for every agent. `다
 * 허용`, the default, runs every request without asking; `에이전트대로` asks
 * as the operator set the agent up, as in the terminal, naming the agent
 * chosen beside it. A choice sends one event; the core holds it.
 */
export function DeskPermissionMenu({ permission, agent, open, onPermission }: {
  permission: DeskPermission;
  /** The chosen agent's name, which `에이전트대로`'s line names; null before one is chosen. */
  agent: string | null;
  /** Opens the menu, for review. */
  open?: boolean;
  onPermission: (permission: DeskPermission) => void;
}) {
  const { t } = useInterfaceTranslation();
  const Chosen = PERMISSION_ICON[permission];
  return (
    <BarMenu name={t("desk.permission.name")} value={<><Chosen aria-hidden="true" className="size-(--size-icon-sm)" />{t(PERMISSION_NAME[permission])}</>} open={open} highlight={open} wide data-desk-bar="permissions" data-desk-permission={permission}>
      <DropdownMenuLabel className={DESK_MENU_HEADING}>{t("desk.permission.heading")}</DropdownMenuLabel>
      <DropdownMenuRadioGroup value={permission} onValueChange={(value) => value !== permission && onPermission(value as DeskPermission)}>
        {PERMISSIONS.map((one) => {
          const Icon = PERMISSION_ICON[one];
          return (
          <DropdownMenuRadioItem key={one} value={one} className="items-start gap-sm" data-desk-permission-choice={one}>
            <Icon aria-hidden="true" className="mt-xxs size-(--size-icon) text-foreground" data-desk-permission-icon={one} />
            <span className="flex min-w-0 flex-col gap-xxs">
              <span className="flex items-center gap-xs text-foreground">
                {t(PERMISSION_NAME[one])}
                {one === "allow_all" ? <Badge variant="outline">{t("desk.permission.default")}</Badge> : null}
              </span>
              <span className="whitespace-normal break-keep text-caption text-subtle-foreground">
                {one === "allow_all" ? t("desk.permission.allow_allDetail") : t("desk.permission.agentDetail", { agent: agent ?? t("desk.bar.agent") })}
              </span>
            </span>
          </DropdownMenuRadioItem>
          );
        })}
      </DropdownMenuRadioGroup>
    </BarMenu>
  );
}
