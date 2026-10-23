import { InterfaceLanguageBoundary, translate, useInterfaceTranslation } from "./i18n/client";
import { agentCapacityNotice } from "./agentLayout";
import { useEffect, useMemo, useRef, type ReactNode } from "react";
import { createActions, type Actions } from "./actions";
import { identity, moveBuffer, tabBufferKey, type BufferKey } from "./buffers";
import { BrowserHost } from "./BrowserDisplay";
import { freshError, watchErrorNotices } from "./errorNotice";
import { DraftRecoveryLine, refreshRecoveryDrafts } from "./DraftRecovery";
import { pruneDrafts, settleDraft } from "./editor/draft";
import { ConnectionBadge } from "./badge";
import { configureFileBytes } from "./fileBytes";
import { installKeyboard, observeRecent, reconcileHeldCycle } from "./keyboard";
import { noteOperatorPointer, observeInputRequests } from "./keyTarget";
import { OverviewModal, OverviewPage } from "./Overview";
import { AgentCloseNotice, ConfirmClose, ConfirmTrash, CycleOverlay, NoticeBar } from "./Overlays";
import { ConfirmArchive, ConfirmArchiveDelete } from "./ArchiveDialogs";
import { QuickAdd } from "./QuickAdd";
import { Palette } from "./Palette";
import { usePracticeChannel } from "./practice/channel";
import { installProbe, probeEnabled } from "./probe";
import { configurePaneVisits, expectPane, expectSurface, focusSignature } from "./recent";
import { FirstRunWizard } from "./FirstRunWizard";
import { HerdrGate } from "./HerdrTakeover";
import { herdrGate } from "./herdrTakeoverRules";
import { useAgentNotifications } from "./agentNotices";
import { useFetchLook, useGithubLook } from "./githubLook";
import { SettingsGate } from "./SettingsSheet";
import { StartPanelHost } from "./StartPanel";
import { GroupNameDialog } from "./AgentGroupControls";
import { InlineStartHost } from "./AgentTabStart";
import { FONT_SIZE_BASE, usableAccent, usableFontSize, usableTerminalFontSize } from "./settings";
import { ShortcutSheet } from "./ShortcutSheet";
import { Sidebar } from "./sidebar";
import { TitleRow } from "./TitleRow";
import { readEditorAvailability } from "./host";
import { RegionFailed, RenderBoundary, ShellFailed, SidebarFailed, TitleRowFailed } from "./RenderBoundary";
import { focusedRemoteDevice, frontCheckout } from "./snapshot";
import { OPEN_ANSWER_TIMEOUT_MS, openingProgress, startupScreen, waitingForSavedWorkspace } from "./navigation";
import { START_ANSWER_TIMEOUT_MS } from "./startAnswer";
import { useShellStore } from "./store";
import { AddProjectDialog } from "./AddProjectDialog";
import { WorkspaceDialogs, WorkspaceNotices } from "./WorkspaceDialogs";
import { applyEditorTheme } from "./editor/theme";
import { applyTerminalFontSize, applyTerminalTheme, attachedPaneIds, feedChunks, liveTerminalIds, resetAllTerminals, retainTerminals, terminalFor, terminalSelectionText } from "./terminals";
import { PALETTE_HIDE_CLASS, primaryValue, readColorTheme, readTheme, resolveTheme } from "./theme";
import { TooltipProvider } from "./components/ui/tooltip";
import { useUsageWindowHint } from "./components/usage";
import { useUiStore } from "./ui";
import { WorkspaceScreen } from "./WorkspaceScreen";
import { connectShell, type DispatchFn } from "./ws";

const noop: DispatchFn = () => {};

export function App() {
  const dispatchRef = useRef<DispatchFn>(noop);
  const actions: Actions = useMemo(() => createActions((event) => dispatchRef.current(event)), []);
  useUsageWindowHint(actions);
  useAgentNotifications(actions);
  useGithubLook(actions);
  useFetchLook(actions);
  usePracticeChannel(actions);
  // Open in VS Code is offered only where the desktop app found it (D-14).
  useEffect(() => readEditorAvailability(), []);

  useEffect(() => {
    const session = connectShell({
      onChunks: (chunks, full) => {
        if (full) resetAllTerminals();
        feedChunks(chunks);
      },
    });
    dispatchRef.current = session.dispatch;
    configureFileBytes(session.dispatch);
    configurePaneVisits(actions.paneVisit);
    const keyboard = installKeyboard(actions);
    if (probeEnabled()) {
      installProbe(
        () => terminalFor(useShellStore.getState().focusedPaneId),
        () => useShellStore.getState().focusedPaneId,
        session.drop,
        attachedPaneIds,
        liveTerminalIds,
        (paneId) => terminalFor(paneId),
        terminalSelectionText,
      );
    }
    // Recent Panels and Recent Projects follow what the operator uses: the
    // core's focus, whichever side moved it, and where the keyboard is, since
    // moving between a checkout's terminal and its View area changes no core
    // state. The operator's next press or click ends a commit's wait.
    // Focus that lands outside both areas (the sidebar, a palette) is not a
    // move between surfaces.
    const observeFocus = (event: FocusEvent) => {
      if (event.target instanceof Element && event.target.closest("[data-agent-area], [data-view-area]")) observeRecent(useShellStore.getState().rest, true);
    };
    // A chord the shell answered (the next cycle) and a bare modifier are
    // not the operator acting on the surface a commit is bringing forward.
    const endCommit = (event: Event) => {
      if (event.defaultPrevented) return;
      if (event instanceof KeyboardEvent && ["Control", "Alt", "Shift", "Meta"].includes(event.key)) return;
      expectSurface(null);
      expectPane(null);
    };
    window.addEventListener("focusin", observeFocus);
    window.addEventListener("pointerdown", endCommit, true);
    // Pointing anywhere is the operator choosing where they are, so keys stop
    // following a pending new tab or split (`keyTarget.ts`).
    window.addEventListener("pointerdown", noteOperatorPointer, true);
    window.addEventListener("keydown", endCommit, true);
    // All projects and an Overview are visits of their own, and leaving one
    // for the Workspace makes the surface there the one in use; neither
    // changes core state, so the page's screen is watched here.
    const unsubscribeScreen = useUiStore.subscribe((state, previous) => {
      if (state.screen !== previous.screen) observeRecent(useShellStore.getState().rest, true);
    });
    // A failure the operator can act on is a notice rather than only a diagnostic (design 13); it is raised
    // before the rest of this subscriber runs, as it always was.
    const unsubscribeNotices = watchErrorNotices();
    const unsubscribe = useShellStore.subscribe((state, previous) => {
      if (state.rest === previous.rest) return;
      // A terminal lives as long as the core streams its pane; released or
      // vanished panes lose theirs here, never on a tab switch (D-05).
      if (state.rest?.terminal?.panes !== previous.rest?.terminal?.panes) {
        retainTerminals();
      }
      if (state.rest?.terminal?.input_requests !== previous.rest?.terminal?.input_requests) observeInputRequests(state.rest?.terminal?.input_requests);
      const fresh = freshError(state, previous);
      const waiting = state.rest?.workspace_view?.agent_layout?.waiting ?? 0;
      const previouslyWaiting = previous.rest?.workspace_view?.agent_layout?.waiting ?? 0;
      if (waiting !== previouslyWaiting || state.rest?.workspace_view?.path !== previous.rest?.workspace_view?.path) {
        if (waiting > 0) useUiStore.getState().setNotice({ text: agentCapacityNotice(waiting), refreshable: false });
        else if (previouslyWaiting > 0 && useUiStore.getState().notice?.text === agentCapacityNotice(previouslyWaiting)) useUiStore.getState().setNotice(null);
      }
      // A refused commit brings nothing forward, so the next surface in use is a visit again.
      if (fresh) {
        expectSurface(null);
        expectPane(null);
      }
      if (state.rest?.navigator === previous.rest?.navigator && state.rest?.workspace_view === previous.rest?.workspace_view && state.rest?.focused === previous.rest?.focused) return;
      observeRecent(state.rest, focusSignature(state.rest) !== focusSignature(previous.rest));
      const cycle = useUiStore.getState().cycle;
      if (cycle) useUiStore.getState().setCycle(reconcileHeldCycle(cycle, state.rest));
    });
    return () => {
      window.removeEventListener("focusin", observeFocus);
      window.removeEventListener("pointerdown", endCommit, true);
      window.removeEventListener("pointerdown", noteOperatorPointer, true);
      window.removeEventListener("keydown", endCommit, true);
      unsubscribeScreen();
      unsubscribeNotices();
      unsubscribe();
      keyboard();
      configurePaneVisits(() => {});
      session.close();
    };
  }, [actions]);

  // Appearance is the core's: the stored accent drives the primary token and
  // the stored interface size scales the interface text tokens, so a reload
  // restores both from the snapshot (B4). A value outside what the sheet
  // offers leaves the token default rather than drawing a guess.
  const accentHex = useShellStore((s) => usableAccent(s.rest?.ui_state?.accent_hex));
  const fontSize = useShellStore((s) => usableFontSize(s.rest?.ui_state?.font_size));
  useEffect(() => {
    const root = document.documentElement.style;
    const primary = primaryValue(accentHex);
    if (primary) root.setProperty("--primary", primary);
    else root.removeProperty("--primary");
    if (fontSize) root.setProperty("--interface-scale", String(fontSize / FONT_SIZE_BASE));
    else root.removeProperty("--interface-scale");
  }, [accentHex, fontSize]);
  const terminalFontSize = useShellStore((s) => usableTerminalFontSize(s.rest?.ui_state?.terminal_font_size));
  useEffect(() => applyTerminalFontSize(terminalFontSize), [terminalFontSize]);

  // The theme is the core's too (D-14, D-15): System follows the OS
  // appearance while it changes, and the terminals are re-colored in place.
  // The color theme rides on the same root as one more class
  // (hide-color-theme D-10), so a change of either re-colors once.
  const storedTheme = useShellStore((s) => s.rest?.ui_state?.theme);
  const storedColorTheme = useShellStore((s) => s.rest?.ui_state?.color_theme);
  useEffect(() => {
    const { choice, unknown } = readTheme(storedTheme);
    if (unknown) useShellStore.getState().noteDiagnostic("ui_state.theme is not one this page knows; Dark is shown");
    const palette = readColorTheme(storedColorTheme);
    if (palette.unknown) useShellStore.getState().noteDiagnostic("ui_state.color_theme is not one this page knows; Hide is shown");
    const query = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      const theme = resolveTheme(choice, query.matches);
      const root = document.documentElement.classList;
      root.toggle("dark", theme === "dark");
      root.toggle("light", theme === "light");
      root.toggle(PALETTE_HIDE_CLASS, palette.choice === "hide");
      applyTerminalTheme();
      applyEditorTheme();
    };
    apply();
    if (choice !== "system") return;
    query.addEventListener("change", apply);
    return () => query.removeEventListener("change", apply);
  }, [storedTheme, storedColorTheme]);

  // A reconnect cancels an in-flight drag or cycle; the registration text
  // stays because its component is not remounted (PRD S2 B14).
  const connection = useShellStore((s) => s.connection);
  useEffect(() => {
    if (connection !== "live") useUiStore.getState().setCycle(null);
  }, [connection]);

  // A save mark belongs to a tab that is still unsaved: once the core reports
  // the tab clean, the mark comes off whichever tab is showing (D-10).
  const editorTabs = useShellStore((s) => s.editor?.tabs);
  const identities = useRef(new Map<string, BufferKey>());
  const host = useShellStore((s) => s.daemon?.host_id ?? null);
  useEffect(() => {
    if (!editorTabs) return;
    const rest = useShellStore.getState().rest;
    const open = new Set<string>();
    for (const tab of editorTabs) {
      if (tab.kind !== "file") continue;
      open.add(tab.id);
      const key = tabBufferKey(host, rest, tab);
      if (!key) continue;
      const before = identities.current.get(tab.id);
      identities.current.set(tab.id, key);
      // A rename, a move, or a device checkout confirmed at its repository
      // root retargets the stored draft to the new identity, showing tab or
      // background tab alike (D-14).
      if (before && identity(before) !== identity(key)) {
        void moveBuffer(before, key).then((outcome) => {
          if (outcome === "failed" && tab.dirty) useShellStore.getState().noteBufferWarning(tab.id, true);
        });
      }
    }
    for (const tabId of [...identities.current.keys()]) {
      if (!open.has(tabId)) identities.current.delete(tabId);
    }
    // A tab id names a path, so a buffer for a tab that is gone would be
    // applied to whatever opens at that path next (B5, D-14).
    pruneDrafts(open);
    for (const tab of editorTabs) if (!tab.dirty) settleDraft(tab.id);
    const state = useShellStore.getState();
    if (state.savingTabs.size === 0) return;
    const dirty = new Set(editorTabs.filter((tab) => tab.dirty).map((tab) => tab.id));
    for (const tabId of state.savingTabs) if (!dirty.has(tabId)) state.noteSaving(tabId, false);
  }, [editorTabs, host]);

  // Drafts no open tab stands for are recovery items: a daemon restart that
  // lost its tabs, another device's or host's documents, or drafts from
  // before drafts named their device. None is discarded on its own (B10-B12).
  const openDocuments = useShellStore((s) => (s.editor?.tabs ?? []).filter((tab) => tab.kind === "file").map((tab) => tab.id).join("\n"));
  useEffect(() => {
    if (connection !== "live") return;
    void refreshRecoveryDrafts();
  }, [connection, openDocuments, host]);

  // Each part fails on its own and this component, which owns the socket,
  // stays mounted (`RenderBoundary`). A notice, an overlay or the browser
  // displays' bridge that cannot draw is left out on its own, since nothing
  // in its place would be something to act on.
  const notices: [string, ReactNode][] = [
    ["connection", <ConnectionBadge actions={actions} />],
    ["notice", <NoticeBar actions={actions} />],
    ["agent_close_notice", <AgentCloseNotice actions={actions} />],
    ["draft_recovery", <DraftRecoveryLine actions={actions} />],
    ["workspace_notices", <WorkspaceNotices actions={actions} />],
  ];
  const overlays: [string, ReactNode][] = [
    ["overview", <OverviewModal actions={actions} />],
    ["cycle", <CycleOverlay />],
    ["confirm_close", <ConfirmClose actions={actions} />],
    ["confirm_trash", <ConfirmTrash actions={actions} />],
    ["confirm_archive", <ConfirmArchive actions={actions} />],
    ["confirm_archive_delete", <ConfirmArchiveDelete actions={actions} />],
    ["palette", <Palette actions={actions} />],
    ["quick_add", <QuickAdd actions={actions} />],
    ["start_panel", <StartPanelHost actions={actions} />],
    ["group_name", <GroupNameDialog actions={actions} />],
    ["inline_starts", <InlineStartHost />],
    ["shortcuts", <ShortcutSheetGate actions={actions} />],
    ["settings", <SettingsGate actions={actions} />],
    ["first_run", <FirstRunWizard actions={actions} />],
    ["workspace_dialogs", <WorkspaceDialogs actions={actions} />],
    ["add_project", <AddProjectDialog actions={actions} />],
    ["browser", <BrowserHost actions={actions} />],
  ];
  // A Herdr on another protocol holds all of Hide: the screen that says so is drawn in place of the shell, so no
  // window, menu item or key opens a workspace on a server Hide cannot use (PRD herdr-takeover).
  const gate = useShellStore((s) => herdrGate(s.rest));
  // A failure of something tried on that server, before the screen came or while it stood, is not one to act on
  // once it goes.
  useEffect(() => {
    useUiStore.getState().setNotice(null);
  }, [gate]);
  if (gate) {
    return (
      <TooltipProvider>
        <InterfaceLanguageBoundary />
        <RenderBoundary region="herdr_gate" fallback={<ShellFailed />}>
          <div className="flex h-full flex-col bg-background text-foreground" data-herdr-gate-screen={gate}>
            <header className="window-drag h-(--size-tab-strip) shrink-0" />
            <HerdrGate actions={actions} gate={gate} />
          </div>
        </RenderBoundary>
      </TooltipProvider>
    );
  }
  const leftOut = ([region, node]: [string, ReactNode]) => (
    <RenderBoundary key={region} region={region} fallback={null}>
      {node}
    </RenderBoundary>
  );
  return (
    <TooltipProvider>
      <InterfaceLanguageBoundary />
      <RenderBoundary region="shell" fallback={<ShellFailed />}>
        <div className="relative flex h-full flex-col bg-background text-foreground">
          <RenderBoundary region="title" fallback={<TitleRowFailed />}>
            <TitleRow actions={actions} />
          </RenderBoundary>
          {notices.map(leftOut)}
          <div className="flex min-h-0 flex-1">
            <RenderBoundary region="sidebar" fallback={<SidebarFailed />}>
              <Sidebar actions={actions} />
            </RenderBoundary>
            <main className="flex min-h-0 min-w-0 flex-1 flex-col">
              <RenderBoundary region="screen" fallback={<RegionFailed />}>
                <CenterScreen actions={actions} />
              </RenderBoundary>
            </main>
          </div>
          {overlays.map(leftOut)}
        </div>
      </RenderBoundary>
    </TooltipProvider>
  );
}

function ShortcutSheetGate({ actions }: { actions: Actions }) {
  const open = useUiStore((s) => s.overlay === "shortcuts");
  return open ? <ShortcutSheet actions={actions} /> : null;
}

/**
 * All projects, a Project's Overview, or the front Workspace (PRD S6 D-02, D-11). The
 * page starts on the Workspace the core kept in front when it is the one used
 * last and still in the catalog, and on All projects on a first run or when it is
 * gone; while the device in front is still being reached the choice waits,
 * so a Workspace that is about to appear does not first flash All projects. A
 * Workspace that goes away while it is shown gives way to All projects.
 */
function CenterScreen({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const screen = useUiStore((s) => s.screen);
  const front = useShellStore((s) => frontCheckout(s.rest)?.id ?? null);
  const hasView = useShellStore((s) => Boolean(s.rest?.workspace_view));
  const first = useShellStore((s) => startupScreen(s.rest, front !== null));
  const restoring = useShellStore((s) => waitingForSavedWorkspace(s.rest) && s.rest?.status?.herdr?.state === "connected");
  const loaded = useShellStore((s) => s.rest !== null);
  const remoteFront = useShellStore((s) => focusedRemoteDevice(s.rest) !== null);
  const opening = useUiStore((s) => s.opening);
  const progress = useShellStore((s) => (opening && !opening.failure ? openingProgress(s.rest, opening) : null));
  // An open from All projects, an Overview or the Agents list shows its Workspace
  // once the core has moved there; a refusal or no answer stays put and says
  // why on All projects or the Overview. On a Workspace the core's error notice
  // already says it, so nothing is kept for later (B2, B21).
  useEffect(() => {
    if (!opening || opening.failure || progress === null) return;
    const store = useUiStore.getState();
    if (progress === "cancelled") {
      store.setOpening(null);
      return;
    }
    if (progress === "landed") {
      // A Workspace already in front keeps an Overview the operator opened
      // over it while the core was still answering: the layer is theirs, not
      // the open's. From All projects or an Overview screen the Workspace
      // comes forward in its place.
      if (store.screen?.kind === "workspace") store.setOpening(null);
      else store.setScreen({ kind: "workspace" });
      return;
    }
    store.setOpening(store.screen?.kind === "workspace" ? null : { ...opening, failure: progress });
  }, [opening, progress]);
  useEffect(() => {
    if (!opening || opening.failure) return undefined;
    const timer = window.setTimeout(() => {
      const store = useUiStore.getState();
      if (store.opening !== opening) return;
      // The start panel owns a missing answer. Presentation may also have
      // been superseded by a core visit, which needs no second notice.
      if ("startRequestId" in opening.target) return store.setOpening(null);
      const failure = translate("shell.openTimeout");
      if (store.screen?.kind !== "workspace") return store.setOpening({ ...opening, failure });
      store.setOpening(null);
      store.setNotice({ text: translate("shell.notOpened", { reason: failure }), refreshable: false });
    }, "startRequestId" in opening.target ? START_ANSWER_TIMEOUT_MS : OPEN_ANSWER_TIMEOUT_MS);
    return () => window.clearTimeout(timer);
  }, [opening]);
  useEffect(() => {
    if (screen !== null || !loaded) return undefined;
    if (first) {
      useUiStore.getState().setScreen({ kind: first });
      return undefined;
    }
    // A connected Herdr's saved choice is still being restored, rather than
    // an unanswered connection. Its catalog/View decides when it settles.
    if (restoring) return undefined;
    // A Herdr or a device that never answers must not hold the page on a
    // blank screen; a device gets longer, since it is reached over SSH.
    const timer = window.setTimeout(() => {
      if (useUiStore.getState().screen === null) useUiStore.getState().setScreen({ kind: "main" });
    }, remoteFront ? 15_000 : 3000);
    return () => window.clearTimeout(timer);
  }, [screen, loaded, first, remoteFront, restoring]);
  if (screen === null) {
    return (
      <div className="flex flex-1 items-center justify-center text-caption text-muted-foreground" data-center-screen="starting">
        {t("shell.openingLastWorkspace")}
      </div>
    );
  }
  if (screen.kind === "workspace" && front && hasView) return <WorkspaceScreen actions={actions} />;
  return <OverviewPage actions={actions} />;
}
