import { AtSignIcon } from "lucide-react";
import { useRef, useState } from "react";
import type { Actions } from "./actions";
import { AgentMark } from "./AgentMark";
import { agentReference, mentionCandidates } from "./agentReference";
import { Button } from "./components/ui/button";
import { Command, CommandEmpty, CommandInput, CommandItem, CommandList } from "./components/ui/command";
import { Popover, PopoverContent, PopoverTrigger } from "./components/ui/popover";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { allAgents } from "./paneAgents";
import { useShellStore } from "./store";
import { focusTerminal, typeReference } from "./terminals";

/**
 * An agent pane's @: the other agents on its device, found by name or task,
 * and the one chosen typed into this pane's prompt as its reference, with no
 * Enter, so the operator only adds what to say. The list is read when it
 * opens; choosing hands the keyboard to this pane's terminal, and Escape back
 * to the @.
 */
export function PaneMention({ paneId, actions, className = "" }: { paneId: string; actions: Actions; className?: string }) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(false);
  // A closed @ reads nothing from the store, so it adds no work to the header's renders.
  const agents = useShellStore((state) => (open ? allAgents(state.agents, state.rest?.status?.remote) : null));
  const rest = useShellStore((state) => (open ? state.rest : null));
  const chosen = useRef(false);
  const candidates = agents ? mentionCandidates(rest, agents, paneId) : [];
  const choose = (reference: string) => {
    chosen.current = true;
    setOpen(false);
    actions.focusPane(paneId);
    focusTerminal(paneId);
    typeReference(paneId, reference);
  };
  const label = t("panes.mention.button");
  // `className` may hide a closed @; an open one stays drawn while the pointer is on its list.
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <Hint label={label} active={!open}>
        <PopoverTrigger asChild>
          <Button
            variant="ghost"
            size="icon-sm"
            className={`shrink-0 text-subtle-foreground hover:bg-popover hover:text-foreground data-[state=open]:bg-secondary data-[state=open]:text-foreground ${open ? "" : className}`}
            aria-label={label}
            data-pane-mention={paneId}
          >
            <AtSignIcon aria-hidden="true" />
          </Button>
        </PopoverTrigger>
      </Hint>
      <PopoverContent
        align="end"
        className="w-(--size-pr-popover) p-none"
        data-pane-mention-list={paneId}
        onOpenAutoFocus={() => {
          chosen.current = false;
        }}
        onCloseAutoFocus={(event) => {
          if (chosen.current) event.preventDefault();
        }}
      >
        <Command>
          <CommandInput placeholder={t("panes.mention.search")} autoFocus />
          <CommandList>
            <CommandEmpty>{t("panes.mention.empty")}</CommandEmpty>
            {candidates.map((agent) => (
              <CommandItem
                key={agent.pane_id}
                value={[agent.identity_label, agent.herdr_name, agent.agent_kind, agent.checkout_label, agent.pane_id].filter(Boolean).join(" ")}
                onSelect={() => choose(agentReference(agent))}
                data-pane-mention-choice={agent.pane_id}
              >
                <AgentMark kind={agent.agent_kind} />
                <span className="min-w-0 truncate">{agent.identity_label}</span>
                {agent.checkout_label ? <span className="ml-auto shrink-0 truncate text-caption text-muted-foreground">{agent.checkout_label}</span> : null}
              </CommandItem>
            ))}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}
