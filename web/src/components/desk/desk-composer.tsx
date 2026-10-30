import { ArrowUpIcon, ChevronDownIcon, ImageIcon, PaperclipIcon, SquareIcon, XIcon } from "lucide-react";
import type { ReactNode } from "react";
import type { DeskAgent, DeskAgentId, DeskChoice, DeskSelection } from "../../desk";
import { agentPickable } from "../../desk";
import { useInterfaceTranslation } from "../../i18n/client";
import type { MessageKey } from "../../i18n/catalogs";
import { cn } from "../../lib/utils";
import { AgentMark } from "../../AgentMark";
import { deskAgentName } from "./desk-agent-name";
import { Button } from "../ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuLabel, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuSeparator, DropdownMenuTrigger } from "../ui/dropdown-menu";
import { Popover, PopoverAnchor, PopoverContent } from "../ui/popover";
import { Textarea } from "../ui/textarea";
import { Hint } from "../ui/tooltip";

/** A file the operator attached: an image or a dropped file, sent as its path (D-62, B29). */
export type ComposerAttachment = { id: string; name: string; image: boolean };

/**
 * The box the operator writes in, for Desk and for a thread alike (B7, B29):
 * the words, the attached files as chips above them, 📎, and send, which
 * becomes stop while Desk answers. `menu` is the `@` menu, opened above the
 * box while `menuOpen`; `children` draws under the box (Desk's bar).
 */
export function DeskComposer({ value, label, placeholder, attachments = [], answering = false, focused = false, menu = null, menuOpen = false, onChange, onSend, onStop, onAttach, onRemoveAttachment, onMenuOpenChange, children }: {
  value: string;
  label: string;
  placeholder: string;
  attachments?: readonly ComposerAttachment[];
  /** Desk is answering: send becomes stop (B7). A thread never answers here. */
  answering?: boolean;
  /** Drawn as the keyboard's place, as `답하기` leaves it (B29). */
  focused?: boolean;
  menu?: ReactNode;
  menuOpen?: boolean;
  onChange: (value: string) => void;
  onSend: () => void;
  onStop?: () => void;
  onAttach: () => void;
  onRemoveAttachment: (id: string) => void;
  onMenuOpenChange?: (open: boolean) => void;
  children?: ReactNode;
}) {
  const { t } = useInterfaceTranslation();
  const empty = value.trim() === "" && attachments.length === 0;
  return (
    <div className="flex min-w-0 flex-col gap-xs" data-desk-composer={answering ? "answering" : "idle"}>
      <Popover open={menuOpen} onOpenChange={onMenuOpenChange}>
        <PopoverAnchor asChild>
          <div className={cn("flex min-w-0 flex-col gap-xs rounded-lg border bg-background px-sm py-xs", focused || menuOpen ? "border-ring ring-1 ring-ring" : "border-input")} data-desk-composer-frame={focused ? "focused" : "rest"}>
            {attachments.length > 0 ? (
              <ul className="flex min-w-0 flex-wrap gap-xs" data-desk-attachments={attachments.length}>
                {attachments.map((attachment) => (
                  <li key={attachment.id} className="flex min-w-0 items-center gap-xxs rounded-xs bg-secondary py-xxs pl-xs text-micro text-subtle-foreground">
                    {attachment.image ? <ImageIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" /> : <PaperclipIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />}
                    <span className="max-w-(--size-tab-title-min) truncate">{attachment.name}</span>
                    <Button variant="ghost" size="icon-xs" aria-label={t("desk.composer.removeAttachment", { name: attachment.name })} onClick={() => onRemoveAttachment(attachment.id)}><XIcon /></Button>
                  </li>
                ))}
              </ul>
            ) : null}
            <div className="flex min-w-0 items-end gap-xs">
              <Textarea bare rows={1} value={value} aria-label={label} placeholder={placeholder} className="max-h-(--size-search-sheet-h) min-h-(--size-control-sm) py-xxs" autoFocus={focused} onChange={(event) => onChange(event.target.value)} />
              <Hint label={t("desk.composer.attach")}>
                <Button variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={t("desk.composer.attach")} onClick={onAttach}><PaperclipIcon /></Button>
              </Hint>
              {answering ? (
                <Hint label={t("desk.composer.stopHint")}>
                  <Button variant="secondary" size="icon-sm" aria-label={t("desk.composer.stop")} data-desk-composer-stop="true" onClick={onStop}><SquareIcon className="fill-current" /></Button>
                </Hint>
              ) : (
                <Hint label={t("desk.composer.send")} shortcut="↵">
                  <Button variant="soft" size="icon-sm" aria-label={t("desk.composer.send")} disabled={empty} data-desk-composer-send="true" onClick={onSend}><ArrowUpIcon /></Button>
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
      {children}
    </div>
  );
}

const AGENT_STATE: Record<DeskAgent["state"], MessageKey> = {
  checking: "desk.agentState.checking",
  cli_missing: "desk.agentState.cli_missing",
  ready: "desk.agentState.ready",
  preparing: "desk.agentState.preparing",
  install_failed: "desk.agentState.install_failed",
  node_missing: "desk.agentState.node_missing",
  login_needed: "desk.agentState.login_needed",
  not_installed: "desk.agentState.not_installed",
  no_ask_mode: "desk.agentState.no_ask_mode",
};

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
 * each a menu. An agent that cannot ask before acting is listed dim with its
 * reason and cannot be picked; one that is preparing or failed says so;
 * switching keeps the conversation (B9). Before an agent is chosen only the
 * agent menu shows; before its first session the model and effort read as
 * the agent's default. `open` opens one menu, for review.
 */
export function DeskAgentBar({ agents, selection, open = null, onAgent, onModel, onEffort }: {
  agents: readonly DeskAgent[];
  selection: DeskSelection;
  open?: "agent" | "model" | "effort" | null;
  onAgent: (agent: DeskAgentId) => void;
  onModel: (model: string) => void;
  onEffort: (effort: string) => void;
}) {
  const { t } = useInterfaceTranslation();
  const agent = selection.agent === null ? null : agents.find((one) => one.id === selection.agent);
  if (agent === undefined) throw new Error(`Desk selection names an unknown agent ${selection.agent}`);
  const choice = (choices: readonly DeskChoice[], id: string | null) => (id === null ? t("desk.bar.default") : (choices.find((one) => one.id === id)?.label ?? id));
  return (
    <div className="flex min-w-0 items-center justify-end gap-xxs" data-desk-agent-bar="true">
      {agent && agent.state !== "ready" ? <span className={cn("mr-auto truncate text-micro", agent.state === "preparing" || agent.state === "checking" ? "text-muted-foreground" : "text-warning")} data-desk-agent-bar-state={agent.state}>{t(AGENT_STATE[agent.state])}</span> : null}
      <BarMenu name={t("desk.bar.agent")} value={agent ? <><AgentMark kind={agent.id} />{deskAgentName(t, agent.id)}</> : t("desk.bar.agent")} open={open === "agent" ? true : undefined} highlight={open === "agent"} data-desk-bar="agent">
        <DropdownMenuLabel>{t("desk.bar.agent")}</DropdownMenuLabel>
        <DropdownMenuRadioGroup value={selection.agent ?? undefined} onValueChange={(value) => onAgent(value as DeskAgentId)}>
          {agents.map((one) => {
            const pickable = agentPickable(one);
            const item = (
              <DropdownMenuRadioItem key={one.id} value={one.id} disabled={!pickable} className="gap-sm" data-desk-agent={one.id} data-desk-agent-state={one.state}>
                <AgentMark kind={one.id} />
                <span className="flex-1">{deskAgentName(t, one.id)}</span>
                <span className={cn("text-micro", one.state === "ready" || one.state === "preparing" || one.state === "checking" ? "text-muted-foreground" : "text-warning")}>{t(AGENT_STATE[one.state])}</span>
              </DropdownMenuRadioItem>
            );
            return pickable ? item : <Hint key={one.id} label={t("desk.agentState.noAskModeHint")} side="left"><span>{item}</span></Hint>;
          })}
        </DropdownMenuRadioGroup>
        <DropdownMenuSeparator />
        <DropdownMenuLabel className="font-normal text-muted-foreground">{t("desk.bar.carries")}</DropdownMenuLabel>
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
