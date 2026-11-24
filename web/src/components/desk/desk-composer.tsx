import { ArrowUpIcon, ChevronDownIcon, LoaderCircleIcon, PaperclipIcon, SquareIcon, UploadIcon, XIcon } from "lucide-react";
import { useRef, useState, type ClipboardEvent, type DragEvent, type KeyboardEvent, type ReactNode, type Ref } from "react";
import { MAX_FILE_BYTES } from "../../attachments";
import type { DeskAgent, DeskAgentId, DeskAgentWord, DeskChoice, DeskSelection } from "../../desk";
import { agentPickable, deskAgentWord } from "../../desk";
import { dragHasFiles, traySendable, type ComposerTray, type SentAttachment, type TrayFile } from "../../deskComposer";
import { fileIcon } from "../../fileIcons";
import { useInterfaceTranslation } from "../../i18n/client";
import type { MessageKey } from "../../i18n/catalogs";
import { formatBytes } from "../../i18n/format";
import { requireInterfaceLanguage } from "../../i18n/locale";
import { cn } from "../../lib/utils";
import { AgentMark } from "../../AgentMark";
import { FileMark } from "../file-mark";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuSeparator, DropdownMenuTrigger } from "../ui/dropdown-menu";
import { Popover, PopoverAnchor, PopoverContent } from "../ui/popover";
import { Textarea } from "../ui/textarea";
import { Hint } from "../ui/tooltip";
import type { DeskDraft } from "../../ui";
import { MentionInput, type MentionInputHandle } from "./mention-input";

const MIB = MAX_FILE_BYTES / (1024 * 1024);

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
  const tooLarge = state === "too_large";
  const refused = tooLarge || state === "unreadable";
  const why = tooLarge ? t("desk.composer.tooLarge", { limit: MIB }) : state === "unreadable" ? t("desk.composer.unreadable") : null;
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
 * send (Desk's bar). With `tray`, the box takes files: dropped on it (the
 * frame marks itself as the place to drop while files are dragged over it),
 * pasted into it, or picked with 📎; a send waits while one is still read
 * in or one is past the cap. With `draft`, the box is Desk's, where picked
 * mentions stand as chips (D-85) and `value` is the draft's text; without,
 * it is a plain text box.
 */
export function DeskComposer({ value, draft, inputRef, label, placeholder, tray, answering = false, focused = false, held = false, notice = null, menu = null, menuOpen = false, onChange, onCaret, onKeyDown, onSend, onStop, onMenuOpenChange, children }: {
  value: string;
  /** Desk's draft: its mentions are drawn as chips in the box. */
  draft?: DeskDraft;
  inputRef?: Ref<MentionInputHandle>;
  label: string;
  placeholder: string;
  /** The files attached so far; without it the box takes none. */
  tray?: ComposerTray;
  /** Desk is answering: send becomes stop (B7). A thread never answers here. */
  answering?: boolean;
  /** Drawn as the keyboard's place, as `답하기` leaves it (B29). */
  focused?: boolean;
  /** Send waits: the words have nowhere to go yet, or the last ones are still on their way. */
  held?: boolean;
  /** One line under the box: why the last words were not sent, or why none can be. */
  notice?: string | null;
  menu?: ReactNode;
  menuOpen?: boolean;
  /** The words changed, and where the caret stands in them. */
  onChange: (value: string, caret: number) => void;
  /** The caret moved in the chip box without the words changing. */
  onCaret?: (caret: number) => void;
  /** The box's keys before it takes them, such as the `@` menu's moves; one it prevents is not sent. */
  onKeyDown?: (event: KeyboardEvent<HTMLElement>) => void;
  onSend: () => void;
  onStop?: () => void;
  onMenuOpenChange?: (open: boolean) => void;
  children?: ReactNode;
}) {
  const { t } = useInterfaceTranslation();
  const picker = useRef<HTMLInputElement>(null);
  const [over, setOver] = useState(false);
  const files = tray?.files ?? [];
  const empty = value.trim() === "" && files.length === 0;
  const blocked = empty || held || (files.length > 0 && traySendable(files) === null);
  const keyDown = (event: KeyboardEvent<HTMLElement>) => {
    onKeyDown?.(event);
    // Enter sends and Shift+Enter starts a line; a key that ends an IME composition only commits it, and the `@` menu takes its own Enter.
    if (event.defaultPrevented || event.key !== "Enter" || event.shiftKey || event.nativeEvent.isComposing || answering || menuOpen) return;
    event.preventDefault();
    if (!blocked) onSend();
  };
  // Only a drag of files from the system is taken: text, links and Hide's own rows keep their own drop.
  const dragOver = (event: DragEvent<HTMLElement>) => {
    if (!tray || !dragHasFiles([...event.dataTransfer.types])) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
    setOver(true);
  };
  const dragLeave = (event: DragEvent<HTMLElement>) => {
    if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setOver(false);
  };
  const drop = (event: DragEvent<HTMLElement>) => {
    setOver(false);
    if (!tray || event.dataTransfer.files.length === 0) return;
    event.preventDefault();
    event.stopPropagation();
    tray.add([...event.dataTransfer.files]);
  };
  // A paste that carries files attaches them; words paste as words.
  const paste = (event: ClipboardEvent<HTMLElement>) => {
    if (!tray || event.clipboardData.files.length === 0) return;
    event.preventDefault();
    tray.add([...event.clipboardData.files]);
  };
  return (
    <div className="flex min-w-0 flex-col gap-xs" data-desk-composer={answering ? "answering" : "idle"}>
      <Popover open={menuOpen} onOpenChange={onMenuOpenChange}>
        <PopoverAnchor asChild>
          <div
            className={cn("relative flex min-w-0 flex-col gap-xs rounded-xl border bg-background px-sm py-xs", focused || menuOpen || over ? "border-ring ring-1 ring-ring" : "border-input")}
            data-desk-composer-frame={over ? "drop" : focused ? "focused" : "rest"}
            onDragEnter={dragOver}
            onDragOver={dragOver}
            onDragLeave={dragLeave}
            onDrop={drop}
            onPasteCapture={paste}
          >
            {files.length > 0 ? (
              <ul className="flex min-w-0 flex-wrap gap-sm pt-xs pr-xs" data-desk-attachments={files.length}>
                {files.map((file) => <AttachmentTile key={file.id} name={file.name} size={file.size} preview={file.preview} state={file.state} read={file.read} onRemove={() => tray?.remove(file.id)} />)}
              </ul>
            ) : null}
            {draft ? (
              <MentionInput ref={inputRef} draft={draft} label={label} placeholder={placeholder} autoFocus={focused} className="max-h-(--size-search-sheet-h) min-h-[calc(var(--text-body)*1.5*2+var(--spacing-xxs)*2)]" onChange={onChange} onCaret={onCaret} onKeyDown={keyDown} />
            ) : (
              <Textarea bare rows={1} value={value} aria-label={label} placeholder={placeholder} className="max-h-(--size-search-sheet-h) min-h-[calc(var(--text-body)*1.5*2+var(--spacing-xxs)*2)] py-xxs" autoFocus={focused} onChange={(event) => onChange(event.target.value, event.target.selectionStart)} data-desk-composer-input="true" onKeyDown={keyDown} />
            )}
            <div className="flex min-w-0 items-center gap-xs">
              {tray ? (
                <Hint label={t("desk.composer.attach")}>
                  <Button variant="ghost" size="icon-sm" className="-ml-xxs text-muted-foreground" aria-label={t("desk.composer.attach")} onClick={() => picker.current?.click()} data-desk-composer-attach="true"><PaperclipIcon /></Button>
                </Hint>
              ) : null}
              <span className="flex min-w-0 flex-1 items-center justify-end">{children}</span>
              {answering ? (
                <Hint label={t("desk.composer.stopHint")}>
                  <Button variant="secondary" size="icon-sm" className="rounded-full" aria-label={t("desk.composer.stop")} data-desk-composer-stop="true" onClick={onStop}><SquareIcon className="fill-current" /></Button>
                </Hint>
              ) : (
                <Hint label={t("desk.composer.send")} shortcut="↵">
                  <Button variant="secondary" size="icon-sm" className="rounded-full" aria-label={t("desk.composer.send")} disabled={blocked} data-desk-composer-send="true" onClick={onSend}><ArrowUpIcon /></Button>
                </Hint>
              )}
            </div>
            {over ? (
              <div className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center gap-xxs rounded-xl bg-background" data-desk-composer-drop="true">
                <span className="flex items-center gap-xs text-caption text-foreground"><UploadIcon aria-hidden="true" className="size-(--size-icon-sm)" />{t("desk.composer.dropHere")}</span>
                <span className="text-micro text-muted-foreground">{t("desk.composer.dropLimit", { limit: MIB })}</span>
              </div>
            ) : null}
          </div>
        </PopoverAnchor>
        {menu ? (
          <PopoverContent side="top" align="start" className="p-none" onOpenAutoFocus={(event) => event.preventDefault()} data-desk-mention-popover="true">
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
  no_ask_mode: "desk.agentState.no_ask_mode",
  off: "desk.agentState.off",
};

/** Why an agent the bar lists cannot be picked, on hover. */
const UNPICKABLE_HINT: Partial<Record<DeskAgentWord, MessageKey>> = {
  no_ask_mode: "desk.agentState.noAskModeHint",
  off: "desk.agentState.offHint",
};

/** A word that asks the operator to act is in the warning color; one that is passing or settled is quiet. */
const QUIET_WORDS: ReadonlySet<DeskAgentWord> = new Set(["signed_in", "checking", "preparing", "no_ask_mode", "off"]);

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
function BarMenu({ name, value, open, onOpenChange, highlight = false, children, ...data }: { name: string; value: ReactNode; open?: boolean; onOpenChange?: (open: boolean) => void; highlight?: boolean; children: ReactNode } & Record<`data-${string}`, string>) {
  return (
    <DropdownMenu open={open} onOpenChange={onOpenChange} modal={false}>
      <Hint label={name}>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="sm" className={cn("h-(--size-control-compact) gap-xxs px-xs text-caption font-normal", highlight ? "bg-secondary text-foreground" : "text-subtle-foreground")} aria-label={name} {...data}>
            {value}
            <ChevronDownIcon className="size-(--size-icon-sm) text-muted-foreground" />
          </Button>
        </DropdownMenuTrigger>
      </Hint>
      <DropdownMenuContent align="end" side="top" className="min-w-(--size-tab-preferred)">{children}</DropdownMenuContent>
    </DropdownMenu>
  );
}

/**
 * Under Desk's composer (D-66, B4, B5): the agent, its model and its effort,
 * each a menu. The agent menu lists the agents switched on in Settings ›
 * Agents (D-79), each with the sign-in Settings reads or its state (D-81);
 * one that cannot ask before acting is listed dim with its reason and cannot
 * be picked; switching keeps the conversation (B9). Before an agent is chosen
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
        <DropdownMenuLabel className="flex items-center justify-between gap-sm">
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
