import { paneHeadPicksPane, paneMoveStartTarget } from "./paneMove";
import { botStarter } from "./bots";
import { usePaneMove } from "./PaneMoveControls";
import { keyboardCommandOwner, keyboardInputRevision, useKeyboardOwner } from "./viewFocus";
import type { TFunction } from "i18next";
import { agentWithProfile, markWords } from "./agentRow";
import { CircleAlertIcon, Columns2Icon, EllipsisIcon, GripVerticalIcon, Maximize2Icon, MessageSquareIcon, MoonIcon, TerminalSquareIcon, XIcon } from "lucide-react";
import { memo, useCallback, useEffect, useRef, useState } from "react";
import { useOutsidePressSurface } from "./components/ui/layer";
import type { Actions } from "./actions";
import { refusalText, submitFiles } from "./attachments";
import { agentDrag, referenceLands } from "./agentReference";
import { explorerDrag } from "./explorerDrag";
import { remoteTargetOfPane } from "./remote";
import { localDeviceId } from "./snapshot";
import { agentsByPane } from "./paneAgents";
import { AgentRecordTitle } from "./components/agent-record-title";
import { AgentMark } from "./AgentMark";
import { PaneLetterButton } from "./components/letter-mark";
import { LookButton, StatusMark } from "./components/status-mark";
import { Separator } from "./components/ui/separator";
import { Button } from "./components/ui/button";
import { Hint } from "./components/ui/tooltip";
import { PaneConversation } from "./PaneConversation";
import { useUiStore } from "./ui";
import { PaneDeskReturn } from "./DeskScreen";
import { PaneAskMark, PaneForkMark, PaneOpenAsksMark, PaneHeadChildren, PaneHeaderBand, PanePrChip, PaneRecordButton, usePaneAgents } from "./PaneHeaderBand";
import { PaneWorkChip } from "./PaneMissionChip";
import { ToolMark } from "./components/tool-mark";
import { PaneViews, usePaneViewRows } from "./ViewComments";
import { PaneConnectionChip } from "./PaneConnection";
import { PaneMention } from "./PaneMention";
import { AncestorPath, usePaneMenu, type PaneHeadOffer, type TerminalMenuContext } from "./PaneRelations";
import { workspaceViewOf } from "./workspace";
import { pairCellShown, pairLabelKey, pairOffer } from "./pairView";
import { chordLabel, commandLabel } from "./shortcutLabels";
import { keySystem } from "./host";
import { useInterfaceTranslation } from "./i18n/client";
import { modChord, TERMINAL_COPY, TERMINAL_PASTE } from "./shortcuts";
import { wakeFailureReason, wakingLine } from "./sleep";
import type { AgentSleep, AgentStatusCode, PaneRow, RowMark, TerminalPane } from "./snapshot";
import { PaneNoticeBody } from "./PaneNotice";
import { caughtUp } from "./operatorFocus";
import { useShellStore } from "./store";
import { attachTerminal, focusTerminal, pasteExplorerCopy, pasteExplorerFile, pasteText, requestView, setTextScale, terminalLinkAt, terminalSelectionText, typeReference } from "./terminals";

/**
 * Transport states with a live stream; anything else is drawn as a caption in
 * the header.
 */
const LIVE_STATES = new Set(["connected", "controlling", "observing", "idle"]);

export function paneTitle(pane: PaneRow): string {
  // The device is already on screen, so the header names the pane by the
  // id its own machine gives it.
  return pane.identity_label ?? pane.terminal_title ?? pane.engine_label ?? pane.device_pane_id;
}

/**
 * The caption a non-live transport state gets, and whether a click asks the
 * core to reattach. `reconnect_pane` finds its pane among this machine's, so a
 * remote pane's caption only reports; the core reattaches it on the host's
 * next session update.
 */
export function transportCaption(transport: TerminalPane | undefined, t: TFunction<"translation">, local = true, offline = false): { text: string; reconnects: boolean } | null {
  // With its host's connection down, a remote attach ends as `closing`; the
  // pane is not closing, its device is unreachable.
  if (offline) return { text: t("panes.transport.disconnected"), reconnects: false };
  if (!transport || LIVE_STATES.has(transport.transport_state)) return null;
  if (!local) {
    const state = transport.transport_state;
    return { text: state === "closing" ? t("panes.transport.closing") : state === "ended" ? t("panes.transport.remoteEnded") : state === "unavailable" ? t("panes.transport.remoteUnavailable") : t("panes.transport.starting"), reconnects: false };
  }
  switch (transport.transport_state) {
    case "released":
      return { text: t("panes.transport.released"), reconnects: true };
    case "ended":
      return {
        text: transport.exit_code == null ? t("panes.transport.ended") : t("panes.transport.exited", { exitCode: transport.exit_code }),
        reconnects: true,
      };
    case "unavailable":
      return { text: t("panes.transport.unavailable"), reconnects: true };
    case "closing":
      return { text: t("panes.transport.closing"), reconnects: false };
    default:
      return { text: t("panes.transport.starting"), reconnects: false };
  }
}

/** The chords the registry binds here for the terminal menu's commands, read when it opens. */
function terminalMenuChords(): TerminalMenuContext["chords"] {
  return {
    copy: chordLabel(modChord(TERMINAL_COPY, keySystem())),
    paste: chordLabel(modChord(TERMINAL_PASTE, keySystem())),
    find: commandLabel("find_in_pane"),
    splitRight: commandLabel("split_right"),
    splitDown: commandLabel("split_down"),
    zoom: commandLabel("toggle_zoom"),
  };
}

/**
 * What a pane shows in place of its terminal while its agent sleeps (PRD
 * agent-sleep B11-B14): the shell under it is not what the operator was
 * talking to, so the terminal stays hidden until the agent is back.
 */
function SleepBody({ paneId, sleep, actions }: { paneId: string; sleep: AgentSleep; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const now = Date.now();
  return (
    <div
      className="absolute inset-0 flex flex-col items-center justify-center gap-sm bg-card px-lg text-center"
      data-pane-sleep={sleep.state}
    >
      {sleep.state === "sleeping" ? (
        <>
          <MoonIcon className="size-(--size-icon-lg) text-muted-foreground" aria-hidden="true" />
          <p className="text-subhead text-foreground">{t("panes.sleep.sleeping")}</p>
          {sleep.progress ? <p className="max-w-full truncate text-caption text-muted-foreground">{sleep.progress}</p> : null}
          <Button variant="secondary" size="sm" className="mt-xs" data-agent-wake={paneId} onClick={() => actions.wakeAgent(paneId)}>
            {t("panes.sleep.wake")}
          </Button>
        </>
      ) : sleep.state === "waking" ? (
        <>
          <MoonIcon className="size-(--size-icon-lg) text-muted-foreground" aria-hidden="true" />
          <p className="text-subhead text-foreground" role="status">
            {t("panes.sleep.waking")}
          </p>
          <p className="text-caption text-muted-foreground">{wakingLine(sleep, now, t)}</p>
        </>
      ) : (
        <>
          <CircleAlertIcon className="size-(--size-icon-lg) text-destructive" aria-hidden="true" />
          <p className="text-subhead text-foreground">{t("panes.sleep.resumeFailed")}</p>
          {sleep.reason ? (
            <p className="max-w-full text-caption text-muted-foreground" role="alert">
              {wakeFailureReason(sleep, t)}
            </p>
          ) : null}
          <div className="mt-xs flex items-center gap-sm">
            <Button size="sm" variant="secondary" data-agent-wake-retry={paneId} onClick={() => actions.wakeAgent(paneId)}>
              {t("common.retry")}
            </Button>
            <Button size="sm" variant="outline" data-agent-wake-fresh={paneId} onClick={() => actions.wakeAgent(paneId, true)}>
              {t("panes.sleep.newSession")}
            </Button>
          </div>
        </>
      )}
    </div>
  );
}

export const PaneView = memo(function PaneView({
  pane,
  transport,
  focused,
  scale,
  actions,
  agentKind,
  markSymbol,
  markStatus = null,
  markTone,
  lettersWaiting = 0,
  lettersUnreadable = false,
  paneCount,
  zoomed,
  local = true,
  keysStayOnDesk = false,
  entryInput = null,
  entryKeys = "pane",
  offline = false,
  owned = false,
}: {
  pane: PaneRow;
  transport: TerminalPane | undefined;
  focused: boolean;
  scale: number;
  actions: Actions;
  /** The pane's agent, as the sidebar row and the tab draw it; null for a plain shell. */
  agentKind: string | null;
  markSymbol: RowMark | null;
  /** The agent's status code, whose words the mark says on hover (status-marks B36). */
  markStatus?: AgentStatusCode | null;
  markTone: string;
  /** Letters the core shows waiting on this pane for a possible draft or an unreadable composer; 0 draws no pill. */
  lettersWaiting?: number;
  /** They wait because the pane is too narrow, or covered, for its composer to be read: the pill says so and offers no press. */
  lettersUnreadable?: boolean;
  /** Panes in the tab, a zoomed pane's hidden siblings included. */
  paneCount: number;
  /** The tab is zoomed, so this pane is the only one drawn. */
  zoomed: boolean;
  /** False for a pane on a selected SSH device. */
  local?: boolean;
  /** Pair view's left pane taken while pair view was on screen: a desk the keys are on keeps them (D-09). */
  keysStayOnDesk?: boolean;
  /** For the pane focus mode was asked for: the keyboard's input count then; arriving, it takes the keys only while the operator has not moved them since (`pairEntry`). */
  entryInput?: number | null;
  /** For that pane: `desk` when a View's chip asked for focus mode, so the View takes the keys and the pane leaves them (`pairEntry`). */
  entryKeys?: "pane" | "desk";
  /** True while that device's connection is down. */
  offline?: boolean;
  /** True where a screen owns the pane's life, as an automation's run terminal (PRD automations B16): its head carries no ⋯ menu and no ✕. */
  owned?: boolean;
}) {
  const { t } = useInterfaceTranslation();
  const hostRef = useRef<HTMLDivElement>(null);
  // A press on the terminal closes an open popover, though the terminal stops its `mousedown`.
  const outsideSurface = useOutsidePressSurface();
  const setHost = useCallback((node: HTMLDivElement | null) => {
    hostRef.current = node;
    outsideSurface(node);
  }, [outsideSurface]);
  const viewGeneration = useShellStore((s) => s.viewGeneration);
  const refusal = useShellStore((s) => s.attachmentRefusal);
  const setRefusal = useShellStore((s) => s.setAttachmentRefusal);
  const paneId = pane.id;
  const owner = useKeyboardOwner();
  const input = owner.kind === "pane" && owner.paneId === paneId;
  // The grip, ◧ and the zoom show on the pane that holds the keyboard, and on any other while it is pointed at.
  const tools = input ? "" : "opacity-0 group-hover/header:opacity-100 group-focus-within/header:opacity-100 hoverless:opacity-100";
  const dispatch = actions.dispatch;
  const title = paneTitle(pane);
  const recordAgent = agentsByPane(usePaneAgents()).get(paneId);
  const selected = useUiStore((s) => s.paneConversation?.paneId === paneId);
  const onScreen = useUiStore((s) => s.screen?.kind === "workspace" && !s.overviewOpen);
  const conversation = selected && onScreen;
  const conversationChord = useShellStore((s) => commandLabel("desk_terminal", s.rest?.ui_state));
  const focusBody = () => {
    if (conversation) document.querySelector<HTMLTextAreaElement>("[data-pane-conversation] [data-desk-composer-input]")?.focus();
    else focusTerminal(paneId);
  };
  // The logo's name, with the profile its badge marks (PRD agent-profiles B17).
  const agentName = agentKind ? agentWithProfile(t, { agent_kind: agentKind, profile: recordAgent?.profile, tool: recordAgent?.tool }) : "";
  const look = markSymbol === "look" ? recordAgent?.look ?? null : null;
  const move = usePaneMove();
  const moving = move?.mutationBlocked(pane.id) ?? false;
  const titleContent = <span data-pane-move-title="true" className="flex min-w-0 items-center">{recordAgent && recordAgent.identity_label === title ? <AgentRecordTitle agent={recordAgent} /> : <span className="min-w-0 truncate">{title}</span>}</span>;
  // What ◧ offers for this pane (`pairOffer`), which the head's split button and the menus read (PRD lane-desk-pair-view B19, D-43, D-53).
  const pair = useShellStore((s) => pairOffer(workspaceViewOf(s.rest), paneId));
  const head: PaneHeadOffer = { mention: Boolean(agentKind), pair };
  // `@` is an item of the pane menu (D-32, B21); its list opens under the `…`.
  const [mention, setMention] = useState(false);
  const paneMenu = usePaneMenu(pane, title, actions, head, () => setMention(true));

  // A dropped file or a pasted image stages through hided and reaches the
  // terminal as the core's own `terminal_attachment` (B14).
  const drop = (event: React.DragEvent) => {
    if (conversation) { event.preventDefault(); event.stopPropagation(); return; }
    // A sidebar agent row types its reference, for this agent to address it.
    const agent = agentDrag.current;
    if (agent) {
      event.preventDefault();
      event.stopPropagation();
      if (!acceptsAgentDrag()) return;
      actions.focusPane(paneId);
      focusTerminal(paneId);
      typeReference(paneId, agent.text);
      return;
    }
    const item = explorerDrag.current;
    if (item) {
      event.preventDefault();
      event.stopPropagation();
      if (!acceptsExplorerDrag()) return;
      actions.focusPane(paneId);
      focusTerminal(paneId);
      pasteExplorerFile(paneId, item);
      return;
    }
    const files = Array.from(event.dataTransfer.files);
    if (files.length === 0) return;
    event.preventDefault();
    void submitFiles(paneId, files, false);
  };

  const acceptsAgentDrag = () => {
    const agent = agentDrag.current;
    return agent !== null && referenceLands(useShellStore.getState().rest, agent.paneId, paneId);
  };

  const acceptsExplorerDrag = () => {
    const item = explorerDrag.current;
    const rest = useShellStore.getState().rest;
    const device = remoteTargetOfPane(rest, paneId) ?? localDeviceId(rest);
    return item !== null && !item.directory && item.device === device;
  };

  // The terminal is shown here and parked on unmount, not disposed: the
  // instance belongs to the pane for as long as the core streams it (D-05).
  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    return attachTerminal(paneId, host, dispatch, actions, useShellStore.getState().rest?.ui_state?.pane_text_scales?.[paneId] ?? 1);
  }, [paneId, dispatch, actions]);

  // After every self-contained snapshot the core may have restarted, so the
  // pane asks for a full frame rather than trusting what it has drawn. The
  // attach already requested what it needed, so the first generation is skipped.
  const mountedGeneration = useRef(viewGeneration);
  useEffect(() => {
    if (viewGeneration === mountedGeneration.current) return;
    mountedGeneration.current = viewGeneration;
    requestView(paneId);
  }, [paneId, viewGeneration]);

  useEffect(() => {
    setTextScale(paneId, scale);
  }, [paneId, scale]);

  // The snapshot's focus moving onto this pane moves DOM focus, only once the
  // snapshot includes the operator's last click; a snapshot that predates it
  // would pull the keys back off the pane just clicked (`operatorFocus.ts`).
  // A move that arrives before then waits for that snapshot, and is dropped
  // when the focus leaves the pane or the operator takes the keys elsewhere
  // first. Catching up is not itself a move: the answer to a click on the
  // pane that was already focused leaves the keys where the operator took
  // them since (issue 608).
  const snapshotIncludesLastClick = useShellStore(caughtUp);
  const wasFocused = useRef(false);
  const waiting = useRef<{ keys: Element | null } | null>(null);
  useEffect(() => {
    const arrived = focused && !wasFocused.current;
    wasFocused.current = focused;
    if (!focused) {
      waiting.current = null;
      return;
    }
    if (!snapshotIncludesLastClick) {
      if (arrived) waiting.current = { keys: document.activeElement };
      return;
    }
    const waited = waiting.current;
    waiting.current = null;
    if (!arrived && waited?.keys !== document.activeElement) return;
    // A palette that holds the keyboard keeps it: a focus that lands late (the
    // core's answer to a choice made in ⌘K) must not take the keys the
    // operator is typing into a palette opened since. The palette hands the
    // keyboard back to the focused pane when it closes (`restoreFocus`).
    if (document.activeElement?.closest("[data-palette]")) return;
    // A desk tab the operator chose in pair view takes its agent's pane to the left and makes it the tab's keyboard
    // pane, so pane commands reach the pane on screen; the keys stay on the desk where the operator pressed. An entry
    // mounts the pair layer anew, so it is never this case and the pane takes the keys.
    if (keysStayOnDesk && keyboardCommandOwner().kind === "view") return;
    // Focus mode arrives with the core's answer: a desk tab or page the operator pressed after asking keeps the keys.
    if (entryInput !== null && keyboardInputRevision() !== entryInput) return;
    // A View's chip asked for focus mode on that View, which takes the keys itself (`viewFocusRequest`).
    if (entryKeys === "desk") return;
    focusBody();
  }, [paneId, focused, snapshotIncludesLastClick, keysStayOnDesk, entryInput, entryKeys, conversation]);

  // Every paste into the pane is the shell's, before xterm's textarea sees it
  // (the listener runs in the capture phase): an image as an attachment, and
  // text as a paste the pane's holder wraps for the program (`pasteText`),
  // since xterm never learns the program's paste mode.
  useEffect(() => {
    const host = hostRef.current;
    if (!host) return undefined;
    const onPaste = (event: ClipboardEvent) => {
      const text = event.clipboardData?.getData("text/plain") ?? "";
      const items = Array.from(event.clipboardData?.items ?? []).filter((item) => item.type.startsWith("image/"));
      const files = items.map((item) => item.getAsFile()).filter((file): file is File => file !== null);
      if (!text && files.length === 0) return;
      event.preventDefault();
      event.stopPropagation();
      if (text && pasteExplorerCopy(paneId, text)) return;
      if (files.length > 0) void submitFiles(paneId, files, true);
      else pasteText(paneId, text);
    };
    host.addEventListener("paste", onPaste, true);
    return () => host.removeEventListener("paste", onPaste, true);
  }, [paneId]);

  const openTerminalMenu = (event: React.MouseEvent) => {
    event.preventDefault();
    paneMenu.openTerminalAt(event.clientX, event.clientY, {
      selection: terminalSelectionText(paneId) !== null,
      zoomed,
      paneCount,
      chords: terminalMenuChords(),
    }, terminalLinkAt(paneId, event));
  };
  const zoomChord = zoomed ? terminalMenuChords().zoom : "";
  const hidden = paneCount - 1;

  const caption = transportCaption(transport, t, local, offline);
  // The core marks only the panes of a machine whose agents sleep: its own,
  // and a node's that dials it, which this canvas draws as a device's.
  const sleep = pane.sleep;
  const header = useShellStore((s) => s.rest?.terminal?.headers?.[paneId]);
  return (
    <section
      className="group/pane @container/pane relative flex h-full min-h-0 min-w-0 flex-col bg-background"
      data-pane-view={paneId}
      data-focused={focused ? "true" : "false"}
      data-menu-open={paneMenu.open ? "true" : "false"}
      data-transport={transport?.transport_state ?? ""}
      onDragOver={(event) => {
        if (agentDrag.current) {
          event.preventDefault();
          event.dataTransfer.dropEffect = acceptsAgentDrag() ? "copy" : "none";
          return;
        }
        if (explorerDrag.current) {
          event.preventDefault();
          event.dataTransfer.dropEffect = acceptsExplorerDrag() ? "copy" : "none";
          return;
        }
        if (event.dataTransfer.types.includes("Files")) event.preventDefault();
      }}
      onDrop={drop}
    >
      <header
        className={`group/header relative flex h-[var(--size-pane-header)] shrink-0 items-center gap-sm pl-sm pr-[calc((var(--size-tab-overflow-control)-var(--size-control-sm))/2)] text-caption ${
          input ? "bg-primary/[var(--opacity-selected-fill)] text-foreground" : "bg-card text-muted-foreground"
        }`}
        onPointerDown={(event) => {
          if (paneMoveStartTarget(event.currentTarget, event.target)) move?.press(paneId, title, event);
        }}
        // The press kept the keyboard where it was in case it became a drag;
        // a click that stayed one picks this pane (a drag swallows its click).
        onClick={(event) => {
          if (!paneHeadPicksPane(event.currentTarget, event.target)) return;
          actions.focusPane(paneId);
          focusBody();
        }}
        onContextMenu={(event) => {
          event.preventDefault();
          if (!owned) paneMenu.openAt(event.clientX, event.clientY);
        }}
      >
        {/* The header's first row is this pane: its state on the left, what can be done to it on the right, each in a place that does not move.
            The left side is one row that gives up its width first and clips at its end, so the right side's controls stay whole at any width;
            below the pane head breakpoints its chips drop their words, then go, and the title keeps what is left.
            The grip says the header moves the pane; it shows while the header is pointed at or holds the keyboard, its place kept. */}
        <div data-pane-move-row="true" className="flex min-w-0 flex-1 items-center gap-sm self-stretch overflow-hidden">
        <Hint label={t("panes.move.grip")}>
          <span data-pane-move-grip="true" className={`-mx-xxs flex shrink-0 cursor-grab text-subtle-foreground ${tools}`}>
            <GripVerticalIcon className="size-(--size-icon-sm)" aria-hidden="true" />
          </span>
        </Hint>
        <AncestorPath pane={pane} actions={actions} />
        {/* The eye is the press that opens what it asks to look at, so it sits beside the record button rather than in it. */}
        {look ? <LookButton label={markWords(t, { mark: "look", status_code: markStatus ?? "idle", look })} onLook={() => actions.openAgent(paneId, look)} data-pane-status-mark="look" /> : null}
        {agentKind ? (
          <PaneRecordButton
            paneId={paneId}
            title={title}
            product={agentName}
            actions={actions}
            plain={
              <>
                {markSymbol && !look ? <StatusMark mark={markSymbol} human={recordAgent?.human_turn !== undefined} label={markStatus ? markWords(t, { mark: markSymbol, status_code: markStatus }) : undefined} className={markTone} data-pane-status-mark={markSymbol} /> : null}
                <Hint label={agentName}><span data-pane-move-excluded="true" className="flex shrink-0"><AgentMark kind={agentKind} bot={botStarter(recordAgent)} profile={recordAgent?.profile} /></span></Hint>
                <Hint label={`${title}\n${agentName}`} reveals>
                  <span data-pane-move-title="true" className="flex min-w-0 flex-1 items-center">{titleContent}<span className="sr-only">, {agentName}</span></span>
                </Hint>
              </>
            }
          >
            {/* The marks are the button's first column, the title its second (`PaneRecordButton`). */}
            <span className="flex items-center gap-sm">
              {markSymbol && !look ? <StatusMark mark={markSymbol} human={recordAgent?.human_turn !== undefined} className={markTone} data-pane-status-mark={markSymbol} /> : null}
              {/* The logo names its product as the plain head's does (agent-mark-quiet B4, B6); the button's own name already says it. */}
              <span data-pane-move-excluded="true" className="flex shrink-0" role="img" aria-label={agentName}><AgentMark kind={agentKind} bot={botStarter(recordAgent)} profile={recordAgent?.profile} /></span>
            </span>
            {titleContent}
            <span className="sr-only">, {agentName}</span>
          </PaneRecordButton>
        ) : (
          <>
            <TerminalSquareIcon className="size-(--size-icon-sm) shrink-0" aria-label={t("agentSessions.shell")} />
            <Hint label={title} reveals>
              <span data-pane-move-title="true" className="min-w-0 truncate">{title}</span>
            </Hint>
          </>
        )}
        {/* A bot in its own space keeps its name at every width too, cut short, since it says why no list of the operator's holds the agent; a 전체 bot's agent is the operator's and wears the bot's owl on its mark instead. */}
        {agentKind && recordAgent?.tool?.owns ? <ToolMark tool={recordAgent.tool} /> : null}
        {/* The fork mark stays at every width: an icon alone, and a side fork is what makes a narrow pane. */}
        {agentKind ? <PaneForkMark pane={pane} actions={actions} /> : null}
        {/* The PR mark gives way below the tight breakpoint, since the session record also holds it; the work chip stays, its mark and count alone, since a Conductor's pane opens its map from it. */}
        {agentKind ? <span className="contents pane-head-tight:hidden"><PanePrChip paneId={paneId} actions={actions} /></span> : null}
        {agentKind ? <PaneWorkChip paneId={paneId} actions={actions} /> : null}
        {/* The views chip gives way below the tight breakpoint too: the View area already shows what it lists. The ask and the letters after it stay, since the operator acts on them here. */}
        {/* The views chip and ◧ are one two-cell button (D-43): the left cell lists what the pane's agent opened and what the
            operator opened from its terminal's links, a shell's included (D-54), the right puts the pane beside its desk, filled
            with the pair view accent while pressed. A pane with nothing to show draws neither. Below the tight breakpoint the
            views cell gives way, below the narrow one ◧ too, which the pane menu then holds; its hint carries ⌘. */}
        <PaneSplit paneId={paneId} name={title} actions={actions} />
        {agentKind ? <PaneAskMark paneId={paneId} header={header} actions={actions} /> : null}
        {agentKind ? <PaneOpenAsksMark paneId={paneId} header={header} actions={actions} /> : null}
        {agentKind && lettersWaiting > 0 ? <PaneLetterButton paneId={paneId} letters={lettersWaiting} unreadable={lettersUnreadable} onDeliver={actions.deliverLettersNow} /> : null}
        <span data-pane-move-blank="true" className="h-full flex-1" />
        </div>
        {/* The zoom shows only on the pane that holds the keyboard or is pointed at, so it leads the right side: the controls
            always drawn stay at its end, and the place it keeps while hidden reads as the title's own margin (B20). */}
        {!zoomed && paneCount > 1 ? <Hint label={t("panes.menu.zoom")}><Button variant="ghost" size="icon-sm" className={`shrink-0 text-subtle-foreground hover:bg-accent ${tools} pane-head-narrow:hidden`} aria-label={t("panes.menu.zoom")} data-pane-zoom="false" disabled={moving} onClick={() => actions.toggleZoom(paneId)}><Maximize2Icon aria-hidden="true" /></Button></Hint> : null}
        {recordAgent?.thread_capability && recordAgent.thread_capability !== "none" ? (
          <Hint label={t(recordAgent.thread_capability === "pending" ? "panes.conversation.pending" : "panes.conversation.show")} shortcut={conversationChord || undefined}>
            <Button variant={selected ? "tint" : "ghost"} size="sm" className="shrink-0 px-xs text-caption aria-disabled:opacity-(--opacity-disabled)" aria-label={t("panes.conversation.show")} aria-pressed={selected} aria-disabled={moving || recordAgent.thread_capability !== "ready"} data-pane-conversation-toggle={paneId} onClick={() => { if (!moving && recordAgent.thread_capability === "ready") actions.togglePaneConversation(paneId); }}>
              <MessageSquareIcon aria-hidden="true" /><span className="pane-head-narrow:hidden">{t("panes.conversation.label")}</span>
            </Button>
          </Hint>
        ) : null}
        <PaneDeskReturn paneId={paneId} actions={actions} />
        {agentKind ? <PaneHeadChildren paneId={paneId} actions={actions} /> : null}
        <PaneConnectionChip pane={pane} agentKind={agentKind} actions={actions} local={local} />
        {zoomed ? (
          <Hint label={zoomChord ? t("panes.unzoomChord", { chord: zoomChord }) : t("panes.unzoom")}>
            <Button
              variant="ghost"
              size="sm"
              className="shrink-0 gap-xxs px-xs text-caption text-foreground hover:bg-accent"
              aria-label={hidden > 0 ? t("panes.unzoomHidden", { count: hidden }) : t("panes.unzoom")}
              disabled={moving}
              data-pane-zoom={hidden}
              onClick={() => {
                actions.toggleZoom(paneId);
                // The chip goes with the zoom; a remote pane stays mounted,
                // so the keyboard returns to its terminal rather than the page.
                focusTerminal(paneId);
              }}
            >
              <Maximize2Icon aria-hidden="true" />
              {hidden > 0 ? <span>+{hidden}</span> : null}
            </Button>
          </Hint>
        ) : null}
        {owned ? null : (
        <>
        <PaneMention paneId={paneId} actions={actions} open={mention} setOpen={setMention} anchor={
        <Hint label={t("panes.actions", { name: title })}>
          <Button
            variant="ghost"
            size="icon-sm"
            className="shrink-0 text-subtle-foreground hover:bg-accent hover:text-foreground"
            aria-haspopup="menu"
            data-pane-menu={paneId}
            onClick={(event) => {
              const button = event.currentTarget.getBoundingClientRect();
              paneMenu.openAt(button.left, button.bottom);
            }}
            onKeyDown={(event) => {
              if (event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey)) {
                event.preventDefault();
                event.currentTarget.click();
              }
            }}
          >
            <EllipsisIcon />
          </Button>
        </Hint>
        } />
        <Hint label={t("panes.close", { name: title })}>
          <Button variant="ghost" size="icon-sm" className="shrink-0 text-subtle-foreground hover:bg-accent hover:text-foreground" disabled={moving} onClick={() => actions.closePane(paneId)}>
            <XIcon />
          </Button>
        </Hint>
        {paneMenu.menu}
        </>
        )}
      </header>
      <div className="h-[var(--size-hairline)] shrink-0 bg-border" />
      <div className="relative min-h-0 flex-1">
        <div ref={setHost} className="absolute inset-0 overflow-hidden" data-terminal-host={paneId} onContextMenu={openTerminalMenu} />
        {refusal?.pane_id === paneId ? (
          <div className="absolute inset-x-0 top-0 flex items-center gap-sm bg-card px-sm py-xxs text-caption text-destructive" data-pane-attachment-refusal="true">
            <span className="min-w-0 flex-1 truncate">{refusalText(refusal.reason, t)}</span>
            <Hint label={t("workspace.dismiss")}>
              <Button variant="ghost" size="icon-sm" className="text-muted-foreground" onClick={() => setRefusal(null)}>
                <XIcon />
              </Button>
            </Hint>
          </div>
        ) : null}
        <PaneHeaderBand paneId={paneId} header={header} actions={actions} />
        {pane.failed_terminal_id ? (
          <div className="absolute inset-0 flex flex-col items-center justify-center gap-sm overflow-auto bg-card px-lg text-center" data-pane-restore-failed={paneId}>
            <CircleAlertIcon className="size-(--size-icon-lg) shrink-0 text-destructive" aria-hidden="true" />
            <p className="text-subhead text-foreground" role="status">{t("panes.restore.failed")}</p>
            <p className="max-w-full break-all text-caption text-muted-foreground">{pane.cwd}</p>
            <p className="max-w-full text-caption text-muted-foreground">{t("panes.restore.explanation")}</p>
            <Button size="sm" variant="outline" onClick={() => actions.closePane(paneId)}>{t("panes.restore.close")}</Button>
          </div>
        ) : sleep ? <SleepBody paneId={paneId} sleep={sleep} actions={actions} /> : null}
        {header?.notice && !caption?.reconnects && !sleep && !pane.failed_terminal_id ? <PaneNoticeBody notice={header.notice} /> : null}
        {caption?.reconnects && !sleep && !pane.failed_terminal_id ? (
          <button
            type="button"
            className="absolute inset-0 flex items-center justify-center bg-card text-caption text-subtle-foreground"
            onClick={() =>
              dispatch({ schema_version: 2, kind: "reconnect_pane", payload: { pane_id: paneId } })
            }
          >
            {caption.text}
          </button>
        ) : null}
        {conversation ? <div className="absolute inset-0 z-10 flex min-h-0 min-w-0 bg-card" data-pane-conversation={paneId}><PaneConversation paneId={paneId} actions={actions} /></div> : null}
      </div>
      {input ? (
        <div aria-hidden="true" className="pointer-events-none absolute inset-0 z-10 border-2 border-primary" data-pane-focus-outline="true" />
      ) : null}
    </section>
  );
});

/**
 * The pane head's views chip and ◧ as one button of two cells (PRD
 * lane-desk-pair-view D-43, B19, B20). ◧ is drawn on a pane with a View to
 * show (its agent's, or one opened from its links, D-54) and, pressed, on
 * the pane pair view draws; every other pane
 * enters pair view from ⌘. and its menus, so its head stays quiet (D-53).
 */
function PaneSplit({ paneId, name, actions }: { paneId: string; name: string; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const offer = useShellStore((s) => pairOffer(workspaceViewOf(s.rest), paneId));
  const cell = useShellStore((s) => pairCellShown(workspaceViewOf(s.rest), paneId));
  const views = usePaneViewRows(paneId).length > 0;
  if (!views && !cell) return null;
  const label = t(pairLabelKey(offer));
  const pressed = offer === "on";
  return (
    // Drawn like the head's other icons, with no box (D-48): two ghost cells and one hairline between them, only the
    // hovered cell filled and ◧ pressed in the pair view accent. Below the narrow breakpoint both cells give way.
    <span data-pane-split={paneId} className="flex shrink-0 items-center gap-xxs pane-head-narrow:hidden">
      {views ? <span className="contents pane-head-tight:hidden"><PaneViews paneId={paneId} name={name} actions={actions} /></span> : null}
      {views && cell ? <Separator orientation="vertical" className="data-[orientation=vertical]:h-(--size-icon) pane-head-tight:hidden" data-pane-split-line="true" /> : null}
      {cell ? (
        <Hint label={label} shortcut={commandLabel("pair_view") || undefined}>
          <Button
            variant="ghost"
            size="icon-sm"
            className={`shrink-0 ${pressed ? "bg-pair-view text-primary-foreground hover:bg-pair-view hover:text-primary-foreground" : "text-subtle-foreground hover:bg-accent"} pane-head-narrow:hidden`}
            aria-label={label}
            aria-pressed={pressed}
            data-pane-pair={offer}
            onClick={() => actions.togglePairView(paneId)}
          >
            <Columns2Icon aria-hidden="true" />
          </Button>
        </Hint>
      ) : null}
    </span>
  );
}
