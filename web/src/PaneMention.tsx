import { AtSignIcon } from "lucide-react";
import { useRef, useState } from "react";
import type { Actions } from "./actions";
import { AgentMark } from "./AgentMark";
import { agentReference, mentionCandidates } from "./agentReference";
import { Button } from "./components/ui/button";
import { Command, CommandEmpty, CommandInput, CommandItem, CommandList } from "./components/ui/command";
import { Popover, PopoverAnchor, PopoverContent } from "./components/ui/popover";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { usePaneAgents } from "./PaneHeaderBand";
import { useShellStore } from "./store";
import { focusTerminal, typeReference } from "./terminals";

/**
 * An agent pane's @: the other agents on its device, found by name or task,
 * and the one chosen typed into this pane's prompt as its reference, with no
 * Enter, so the operator only adds what to say. The list is read when it
 * opens; choosing hands the keyboard back to this pane's terminal.
 */
export function PaneMention({ paneId, actions }: { paneId: string; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(false);
  const agents = usePaneAgents();
  // A closed @ reads nothing, so a snapshot that moves no agent renders no header.
  const rest = useShellStore((state) => (open ? state.rest : null));
  const chosen = useRef(false);
  const candidates = open ? mentionCandidates(rest, agents, paneId) : [];
  const choose = (reference: string) => {
    chosen.current = true;
    setOpen(false);
    actions.focusPane(paneId);
    focusTerminal(paneId);
    typeReference(paneId, reference);
  };
  const label = t("panes.mention.button");
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <Hint label={label}>
        <PopoverAnchor asChild>
          <Button
            variant="ghost"
            size="icon-sm"
            className="shrink-0 text-subtle-foreground hover:bg-popover hover:text-foreground data-[state=open]:bg-secondary data-[state=open]:text-foreground"
            aria-label={label}
            aria-haspopup="dialog"
            aria-expanded={open}
            data-state={open ? "open" : "closed"}
            data-pane-mention={paneId}
            onClick={() => setOpen(!open)}
          >
            <AtSignIcon aria-hidden="true" />
          </Button>
        </PopoverAnchor>
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
