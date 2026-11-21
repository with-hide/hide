import { useRef, type ReactNode } from "react";
import type { Actions } from "./actions";
import { AgentMark } from "./AgentMark";
import { agentReference, mentionCandidates } from "./agentReference";
import { Command, CommandEmpty, CommandInput, CommandItem, CommandList } from "./components/ui/command";
import { useReturnFocus } from "./components/ui/layer";
import { Popover, PopoverAnchor, PopoverContent } from "./components/ui/popover";
import { useInterfaceTranslation } from "./i18n/client";
import { allAgents } from "./paneAgents";
import { useShellStore } from "./store";
import { focusTerminal, typeReference } from "./terminals";

/**
 * An agent pane's @, an item of its pane menu (PRD lane-desk-pair-view
 * D-32, B21): the other agents on its device, found by name or task, and the
 * one chosen typed into this pane's prompt as its reference, with no Enter,
 * so the operator only adds what to say. The list opens under `anchor`, the
 * menu's `…` button, and is read when it opens; choosing hands the keyboard
 * to this pane's terminal, and Escape back to the `…`; a press elsewhere
 * keeps the keyboard where it landed (`useReturnFocus`).
 */
export function PaneMention({ paneId, actions, open, setOpen, anchor }: { paneId: string; actions: Actions; open: boolean; setOpen: (open: boolean) => void; anchor: ReactNode }) {
  const { t } = useInterfaceTranslation();
  // A closed @ reads nothing from the store, so it adds no work to the header's renders.
  const agents = useShellStore((state) => (open ? allAgents(state.agents, state.rest?.status?.remote) : null));
  const rest = useShellStore((state) => (open ? state.rest : null));
  const anchorBox = useRef<HTMLSpanElement>(null);
  const candidates = agents ? mentionCandidates(rest, agents, paneId) : [];
  // The list has no trigger of its own: the keyboard goes back to the `…` it opened under.
  const returnFocus = useReturnFocus(open, { target: () => anchorBox.current?.querySelector("button") ?? null });
  const choose = (reference: string) => {
    setOpen(false);
    actions.focusPane(paneId);
    focusTerminal(paneId);
    typeReference(paneId, reference);
  };
  return (
    <Popover open={open} onOpenChange={setOpen}>
      {/* A box of its own, so the list measures the `…` whatever its hint wraps it in. */}
      <PopoverAnchor asChild><span ref={anchorBox} className="flex shrink-0">{anchor}</span></PopoverAnchor>
      <PopoverContent
        align="end"
        className="w-(--size-pr-popover) p-none"
        data-pane-mention-list={paneId}
        onCloseAutoFocus={returnFocus}
      >
        <Command>
          <CommandInput placeholder={t("panes.mention.search")} autoFocus />
          <CommandList>
            <CommandEmpty>{t("panes.mention.empty")}</CommandEmpty>
            {candidates.map((agent) => (
              <CommandItem
                key={agent.pane_id}
                value={[agent.identity_label, agent.given_name, agent.agent_kind, agent.checkout_label, agent.pane_id].filter(Boolean).join(" ")}
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
