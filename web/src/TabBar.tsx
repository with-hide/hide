import { usePaneMove } from "./PaneMoveControls";
import { movingTab } from "./paneMove";
import { PanelsTopLeftIcon, XIcon } from "lucide-react";
import { memo, useEffect, useRef, useState } from "react";
import type { Actions } from "./actions";
import { StatusMark } from "./components/status-mark";
import { markTone, markWords, recordWords } from "./agentRow";
import { AgentRecordTitle } from "./components/agent-record-title";
import { agentsByPane } from "./paneAgents";
import { usePaneAgents } from "./PaneHeaderBand";
import { AgentMark } from "./AgentMark";
import { useAgentLogosShown } from "./agentLogoMode";
import { agentProductName } from "./agentAdapters";
import { Button } from "./components/ui/button";
import { Input } from "./components/ui/input";
import { Hint, Tooltip, TooltipTrigger, TooltipContent } from "./components/ui/tooltip";
import type { AgentRow, AsyncOperation, Checkout, StripTab, TabAgent } from "./snapshot";
import { useShellStore } from "./store";
import { tabFit, type AreaTabInteraction } from "./AreaTree";
import { useInterfaceTranslation } from "./i18n/client";
import type { TabFit } from "./areaLayout";

// The sole Agent tab rendering unit, reused by every local and device area.
const NONE: AsyncOperation[] = [];

/** Phases of a close the core is still confirming with Herdr (`operations.rs`). */
const IN_FLIGHT = new Set(["transmitting", "awaiting_topology", "unknown"]);

export function AgentDraftTab({ id, interaction, actions }: { id: string; interaction: AreaTabInteraction; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const label = t("groups.newTab");
  return <TabButton entry={{ id, source_id: id, kind: "herdr", label, preview: false }} identity={label} agentPaneCount={0} editor={null} mark={<PanelsTopLeftIcon className="size-(--size-icon-sm) shrink-0" />} active={interaction.selected} areaActive={interaction.areaActive} closing={false} closeLabel={t("shell.closeTab", { title: label })} fit={interaction.fit} dragging={interaction.dragging} onSelect={interaction.select} onClose={() => actions.closeTab(id)} onPointerDown={interaction.press} />;
}

/** "closing…" while the core is still confirming a close with Herdr. */
export function closingSuffix(targetId: string, kind: "tab.close" | "pane.close", operations: AsyncOperation[]): boolean {
  return operations.some((op) => op.kind === kind && op.target_id === targetId && IN_FLIGHT.has(op.phase));
}

export function AgentTab({ entry, checkout, interaction, actions, renaming, onCancelRename }: { entry: StripTab; checkout: Checkout; interaction: AreaTabInteraction; actions: Actions; renaming: boolean; onCancelRename: () => void }) {
  const { t } = useInterfaceTranslation();
  const operations = useShellStore((s) => s.rest?.status?.async_operations) ?? NONE;
  const tab = checkout.tabs.find((row) => row.id === entry.source_id);
  const move = usePaneMove();
  const moving = movingTab(operations, entry.source_id) || !!tab?.panes.some(pane => move?.mutationBlocked(pane.id));
  const agent = tab?.agent;
  const rows = agentsByPane(usePaneAgents());
  const focusedAgent = tab?.sidebar?.focused_pane_id ? rows.get(tab.sidebar.focused_pane_id) : undefined;
  const recordAgent = focusedAgent?.identity_label === entry.label ? focusedAgent : undefined;
  // The core gives only agent panes an identity, on local and device tabs alike.
  // Native subagents live inside that pane and are not additional panes.
  const agentPaneCount = tab?.panes.reduce((count, pane) => count + Number(pane.identity_label !== null), 0) ?? 0;
  const identity = `${agent ? t("panes.agent.tabIdentityAgent", { kind: agentProductName(agent.agent_kind), label: entry.label }) : t("panes.agent.tabIdentityTerminal", { label: entry.label })}${agent ? ` · ${markWords(t, agent)}` : ""}${agentPaneCount > 0 ? ` · ${t("panes.agent.tabPaneCount", { count: agentPaneCount })}` : ""}`;
  return <TabButton recordAgent={recordAgent} entry={entry} editor={renaming ? <TabRenameInput key={entry.source_id} entry={entry} actions={actions} onCancel={onCancelRename} /> : null} identity={[identity, recordAgent && recordWords(t, recordAgent)].filter(Boolean).join(" · ")} agentPaneCount={agentPaneCount} mark={<TabMark agent={agent} />} active={interaction.selected} areaActive={interaction.areaActive} closeDisabled={moving} closing={closingSuffix(entry.source_id, "tab.close", operations)} closeLabel={t("shell.closeTab", { title: entry.label })} fit={interaction.fit} dragging={interaction.dragging} onSelect={interaction.select} onClose={() => actions.closeTab(entry.source_id)} onPointerDown={event => { if (!moving) interaction.press(event); }} />;
}

/**
 * An agent tab's mark: its status, then its logo while logos are shown or an
 * ⌥ hold peeks at them (PRD agent-logo-view-mode B7, B13); a tab with no agent
 * is the neutral `>_` in both modes.
 */
function TabMark({ agent }: { agent: TabAgent | null | undefined }) {
  const shown = useAgentLogosShown();
  const status = agent ? <StatusMark mark={agent.mark} className={markTone(agent)} data-tab-status={agent.status_code} /> : null;
  if (agent && !shown) return status;
  return <>{status}<AgentMark kind={agent?.agent_kind} /></>;
}

function TabRenameInput({ entry, actions, onCancel }: { entry: StripTab; actions: Actions; onCancel: () => void }) {
  const { t } = useInterfaceTranslation();
  const [value, setValue] = useState(entry.label);
  const [requestId, setRequestId] = useState<string | null>(null);
  const sent = useRef(false);
  const input = useRef<HTMLInputElement>(null);
  const receipt = useShellStore((state) => state.rest?.status?.tab_rename);
  const answer = receipt?.request_id === requestId ? receipt : null;
  const failed = answer?.phase === "failed";
  const pending = requestId !== null && !failed;
  useEffect(() => {
    const frame = requestAnimationFrame(() => { input.current?.focus(); input.current?.select(); });
    return () => cancelAnimationFrame(frame);
  }, []);
  useEffect(() => {
    if (answer?.phase === "succeeded") onCancel();
    if (failed) sent.current = false;
  }, [answer?.phase, failed, onCancel]);
  const cancel = () => {
    const tab = input.current?.closest<HTMLElement>("[role=tab]");
    onCancel();
    tab?.focus();
  };
  return (
    <Tooltip open={failed}>
      <TooltipTrigger asChild>
        <Input
          ref={input}
          aria-label={t("panes.agent.tabName")}
          data-renaming="true"
          aria-invalid={failed || undefined}
          aria-describedby={failed ? "tab-rename-failure" : undefined}
          className="h-(--size-control-sm) flex-1 px-xs text-caption"
          value={value}
          readOnly={pending}
          onChange={(event) => setValue(event.target.value)}
          onPointerDown={(event) => event.stopPropagation()}
          onClick={(event) => event.stopPropagation()}
          onBlur={onCancel}
          onKeyDown={(event) => {
            event.stopPropagation();
            if (event.nativeEvent.isComposing || event.keyCode === 229) return;
            if (event.key === "Escape") { event.preventDefault(); cancel(); }
            if (event.key === "Enter" && !sent.current) {
              event.preventDefault();
              sent.current = true;
              const id = crypto.randomUUID();
              setRequestId(id);
              actions.renameTab(entry.source_id, value, id);
            }
          }}
        />
      </TooltipTrigger>
      <TooltipContent side="bottom" align="start" className="max-w-none whitespace-nowrap text-destructive" id="tab-rename-failure" role="status">{t("shell.renameFailed")}</TooltipContent>
    </Tooltip>
  );
}

const TabButton = memo(function TabButton({
  entry,
  recordAgent,
  identity,
  agentPaneCount,
  editor,
  mark,
  active,
  closing,
  closeLabel,
  closeDisabled = false,
  fit: slotFit,
  dragging,
  areaActive,
  onSelect,
  onClose,
  onPointerDown,
}: {
  entry: StripTab;
  recordAgent?: AgentRow;
  identity: string;
  agentPaneCount: number;
  editor: React.ReactNode;
  mark: React.ReactNode;
  active: boolean;
  closing: boolean;
  closeLabel: string;
  closeDisabled?: boolean;
  /** How the tab draws its contents in its slot (`AreaTabInteraction.fit`). */
  fit: TabFit;
  dragging: boolean;
  areaActive: boolean;
  onSelect: () => void;
  onClose: () => void;
  onPointerDown: (event: React.PointerEvent<HTMLElement>) => void;
}) {
  const fit = tabFit(active, slotFit);
  const { t } = useInterfaceTranslation();
  return (
    <Hint label={identity} reveals>
    <div
      role="tab"
      aria-selected={active}
      aria-label={identity}
      tabIndex={0}
      data-tab={entry.source_id}
      data-tab-kind={entry.kind}
      data-closing={closing ? "true" : "false"}
      className={`group relative flex min-w-0 flex-1 cursor-default select-none items-center gap-xs text-caption outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring ${fit.tab} ${
        active ? `text-foreground ${areaActive ? "bg-background" : "bg-secondary"}` : "text-subtle-foreground hover:bg-accent"
      } ${dragging ? "opacity-[var(--opacity-dimmed)]" : ""}`}
      onPointerDown={onPointerDown}
      onClick={onSelect}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onSelect();
        }
      }}
    >
      {mark}
      {editor ?? <span className={`@container/tab-title flex h-full min-w-0 flex-1 items-center gap-xs ${fit.title}`}>
        {recordAgent ? <AgentRecordTitle agent={recordAgent} /> : <span className="min-w-0 flex-1 truncate">{entry.label}</span>}
        {closing ? <span className="text-muted-foreground"> {t("panes.transport.closing")}</span> : null}
        {/* Keep title room and the close slot even at the title minimum.
            CSS follows the available title slot without another resize observer. */}
        {agentPaneCount >= 2 && slotFit === "titled" ? <span aria-hidden="true" data-agent-pane-count={agentPaneCount} className="hidden shrink-0 items-center gap-xxs text-micro text-muted-foreground tabular-nums agent-tab-count-room:inline-flex">
          <svg viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1" className="size-(--size-icon-sm) shrink-0">
            <rect x="1.5" y="2" width="11" height="10" rx="1.5" />
            <path d="M7 2v10M7 7h5.5" />
          </svg>
          {agentPaneCount}
        </span> : null}
      </span>}
      <Hint label={closeLabel}>
        <Button
          variant="ghost"
          size="icon-sm"
          className={`shrink-0 hover:bg-popover hover:text-foreground ${fit.close} focus-visible:visible group-hover:visible ${active ? "visible" : "invisible"}`}
          aria-label={closeLabel}
          disabled={closeDisabled}
          onPointerDown={(event) => event.stopPropagation()}
          onClick={(event) => {
            event.stopPropagation();
            onClose();
          }}
        >
          <XIcon />
        </Button>
      </Hint>
      {active && areaActive ? <span className="absolute inset-x-0 bottom-0 h-[var(--size-tab-indicator)] bg-primary" /> : null}

    </div>
    </Hint>
  );
});
