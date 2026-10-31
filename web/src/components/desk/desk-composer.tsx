import { ArrowUpIcon, ChevronDownIcon, ImageIcon, LoaderCircleIcon, PaperclipIcon, SquareIcon, XIcon } from "lucide-react";
import type { KeyboardEvent, ReactNode, Ref } from "react";
import type { DeskAgent, DeskAgentId, DeskAgentWord, DeskChoice, DeskSelection } from "../../desk";
import { agentPickable, deskAgentWord } from "../../desk";
import { useInterfaceTranslation } from "../../i18n/client";
import type { MessageKey } from "../../i18n/catalogs";
import { cn } from "../../lib/utils";
import { AgentMark } from "../../AgentMark";
import { Button } from "../ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuSeparator, DropdownMenuTrigger } from "../ui/dropdown-menu";
import { Popover, PopoverAnchor, PopoverContent } from "../ui/popover";
import { Textarea } from "../ui/textarea";
import { Hint } from "../ui/tooltip";
import type { DeskDraft } from "../../ui";
import { MentionInput, type MentionInputHandle } from "./mention-input";

/** A file the operator attached: an image or a dropped file, sent as its path (D-62, B29). */
export type ComposerAttachment = { id: string; name: string; image: boolean };

/**
 * The box the operator writes in, for Desk and for a thread alike (B7, B29):
 * the words, the attached files as chips above them, 📎, and send, which
 * becomes stop while Desk answers. `menu` is the `@` menu, opened above the
 * box while `menuOpen`; `children` sits in the box's foot between 📎 and
 * send (Desk's bar). 📎 shows only where attaching is taken (`onAttach`).
 * With `draft`, the box is Desk's, where picked mentions stand as chips
 * (D-85) and `value` is the draft's text; without, it is a plain text box.
 */
export function DeskComposer({ value, draft, inputRef, label, placeholder, attachments = [], answering = false, focused = false, held = false, notice = null, menu = null, menuOpen = false, onChange, onCaret, onKeyDown, onSend, onStop, onAttach, onRemoveAttachment, onMenuOpenChange, children }: {
  value: string;
  /** Desk's draft: its mentions are drawn as chips in the box. */
  draft?: DeskDraft;
  inputRef?: Ref<MentionInputHandle>;
  label: string;
  placeholder: string;
  attachments?: readonly ComposerAttachment[];
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
  onAttach?: () => void;
  onRemoveAttachment?: (id: string) => void;
  onMenuOpenChange?: (open: boolean) => void;
  children?: ReactNode;
}) {
  const { t } = useInterfaceTranslation();
  const empty = value.trim() === "" && attachments.length === 0;
  const blocked = empty || held;
  const keyDown = (event: KeyboardEvent<HTMLElement>) => {
    onKeyDown?.(event);
    // Enter sends and Shift+Enter starts a line; a key that ends an IME composition only commits it, and the `@` menu takes its own Enter.
    if (event.defaultPrevented || event.key !== "Enter" || event.shiftKey || event.nativeEvent.isComposing || answering || menuOpen) return;
    event.preventDefault();
    if (!blocked) onSend();
  };
  return (
    <div className="flex min-w-0 flex-col gap-xs" data-desk-composer={answering ? "answering" : "idle"}>
      <Popover open={menuOpen} onOpenChange={onMenuOpenChange}>
        <PopoverAnchor asChild>
          <div className={cn("flex min-w-0 flex-col gap-xs rounded-xl border bg-background px-sm py-xs", focused || menuOpen ? "border-ring ring-1 ring-ring" : "border-input")} data-desk-composer-frame={focused ? "focused" : "rest"}>
            {attachments.length > 0 ? (
              <ul className="flex min-w-0 flex-wrap gap-xs" data-desk-attachments={attachments.length}>
                {attachments.map((attachment) => (
                  <li key={attachment.id} className="flex min-w-0 items-center gap-xxs rounded-xs bg-secondary py-xxs pl-xs text-micro text-subtle-foreground">
                    {attachment.image ? <ImageIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" /> : <PaperclipIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />}
                    <span className="max-w-(--size-tab-title-min) truncate">{attachment.name}</span>
                    <Button variant="ghost" size="icon-xs" aria-label={t("desk.composer.removeAttachment", { name: attachment.name })} onClick={() => onRemoveAttachment?.(attachment.id)}><XIcon /></Button>
                  </li>
                ))}
              </ul>
            ) : null}
            {draft ? (
              <MentionInput ref={inputRef} draft={draft} label={label} placeholder={placeholder} autoFocus={focused} className="max-h-(--size-search-sheet-h) min-h-(--size-control-sm)" onChange={onChange} onCaret={onCaret} onKeyDown={keyDown} />
            ) : (
              <Textarea bare rows={1} value={value} aria-label={label} placeholder={placeholder} className="max-h-(--size-search-sheet-h) min-h-(--size-control-sm) py-xxs" autoFocus={focused} onChange={(event) => onChange(event.target.value, event.target.selectionStart)} data-desk-composer-input="true" onKeyDown={keyDown} />
            )}
            <div className="flex min-w-0 items-center gap-xs">
              {onAttach ? (
                <Hint label={t("desk.composer.attach")}>
                  <Button variant="ghost" size="icon-sm" className="-ml-xxs text-muted-foreground" aria-label={t("desk.composer.attach")} onClick={onAttach}><PaperclipIcon /></Button>
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
          </div>
        </PopoverAnchor>
        {menu ? (
          <PopoverContent side="top" align="start" className="p-none" onOpenAutoFocus={(event) => event.preventDefault()} data-desk-mention-popover="true">
            {menu}
          </PopoverContent>
        ) : null}
      </Popover>
      {notice ? <p role="status" className="px-xs text-caption text-warning" data-desk-composer-notice="true">{notice}</p> : null}
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
