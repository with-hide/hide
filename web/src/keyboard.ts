// Window-level interception of registry chords, ahead of xterm.
//
// The listener runs in the capture phase on `window`, so a chord is answered
// before xterm's textarea sees it and before Chrome's default (bookmark,
// find, zoom) runs (PRD S2 B12). A keydown during IME composition is never a
// chord: Korean input composes through the same keys.
//
// In the desktop app the same table runs its Electron column, and the app
// menu's commands arrive through the host bridge into the same `run`: a
// chord the listener answers is consumed here, so the menu's own
// accelerator for it never fires as well.
//
// The same listener watches the modifiers for the hold hint (PRD
// electron-digit-shortcuts-hints D-03, D-04): every keydown and keyup feeds
// the pure state in `hints.ts`, one timer advances its deadline, and only
// the family it reveals (`agents`, `devices` or none) is published to the ui
// store, so an unrevealed hold renders nothing. Losing the window, a page
// going hidden, a layer opening or any key pressed during the hold ends it.

import { agentCycle, agentOrigin, areaCycle, focusedCycleScope, focusedSurface, scopedSurfaces } from "./areaCycle";
import type { Actions } from "./actions";
import { focusBrowserDisplay } from "./browserViews";
import { advanceHint, clearHint, holdModifiers, idleHint, modifiersOf, NO_MODIFIERS, revealedFamily, type HintState } from "./hints";
import { browserBridge, CHECK_UPDATES_COMMAND, hostBridge, hostKind, keySystem, SETUP_COMMAND } from "./host";
import { herdrGate } from "./herdrTakeoverRules";
import { useSetupWalk } from "./FirstRunWizard";
import { numberedDevices, numberedTabs, sidebarNumbers } from "./numbering";
import { areaFrame } from "./areaFrames";
import { availableEntries, currentEntry, expectPane, expectSurface, observeEntries, observePane, observeProject, paneItem, panelItem, projectItem, reconcileCycle, recentEntries, recentProjectOrder, type CycleItem } from "./recent";
import { projectsOf, remoteContext } from "./remote";
import { hostChord, hostRegistry, isNumberedCommand, releaseModifier, matchHost, numberedCommand, REGISTRY, storedBindings, type CommandId, type Digit, type NumberedFamily } from "./shortcuts";
import { editorFor, type AgentRow, type SnapshotRest, type Workspace } from "./snapshot";
import { useShellStore } from "./store";
import { coveringLayer, useUiStore, type Cycle } from "./ui";
import { drawnViews, installKeyboardOwner, keyboardCommandOwner, noteCommandDelivered, subscribeKeyboardOwner } from "./viewFocus";

/**
 * Recent Panels: every surface the shell holds on every connected device and
 * every screen of the page's own the operator has been on, most recent first,
 * so index 0 is the one in use and one chord lands on the one before it
 * (PRD home-device-rail D-16). A row on a device other than the one in front
 * carries that device's chip.
 */
export function panelCycle(rest: SnapshotRest | null): Cycle | null {
  const items = recentEntries()
    .map((entry) => panelItem(rest, entry))
    .filter((item): item is CycleItem => item !== null);
  return items.length > 1 ? { kind: "panels", items, index: 0 } : null;
}

/** The projects of every connected device, this machine's first, none of them a device's Home. */
function allProjects(rest: SnapshotRest | null): Workspace[] {
  return [
    ...projectsOf(rest?.navigator?.workspaces ?? []),
    ...(rest?.status?.remote ?? []).filter((status) => status.state === "connected").flatMap((status) => projectsOf(status.session?.workspaces ?? [])),
  ];
}

/** Recent Projects across every connected device, each row naming the surface it restores. */
export function projectCycle(rest: SnapshotRest | null): Cycle | null {
  const workspaces = allProjects(rest);
  const order = recentProjectOrder(workspaces.map((workspace) => workspace.id));
  const items = order
    .map((id) => workspaces.find((workspace) => workspace.id === id))
    .map((workspace) => (workspace ? projectItem(workspace, rest) : null))
    .filter((item): item is CycleItem => item !== null);
  return items.length > 1 ? { kind: "projects", items, index: 0 } : null;
}

/**
 * Brings the recent order up to date with the session, and records what the
 * operator is using now when `moved` says the surface in use may have
 * changed: the Workspace surface or page screen on whichever device is in
 * front.
 */
export function observeRecent(rest: SnapshotRest | null, moved: boolean) {
  const owner = keyboardCommandOwner();
  const surface = focusedSurface(rest, focusedCycleScope(rest, owner));
  observeEntries(rest, moved ? surface ?? currentEntry(useUiStore.getState().screen, rest, owner.kind === "view") : null);
  observePane(rest, moved ? (agentOrigin(rest, owner)?.paneId ?? null) : null);
  if (moved) observeProject(remoteContext(rest)?.session?.focused_workspace_id ?? rest?.navigator?.focused_workspace_id);
}

/** The held cycle once the session changed under it: see `reconcileCycle`. */
export function reconcileHeldCycle(cycle: Cycle, rest: SnapshotRest | null): Cycle | null {
  const membership = cycle.scope ? scopedSurfaces(rest, cycle.scope) : null;
  if (cycle.kind === "area" && !membership) return null;
  const alive = cycle.kind === "area" ? new Set(membership!.surfaces.map((surface) => surface.key)) :
    cycle.kind === "agents"
      ? new Set(cycle.items.filter((item) => item.target.kind === "pane" && paneItem(rest, item.target.paneId)).map((item) => item.key))
      : cycle.kind === "projects"
        ? new Set(allProjects(rest).map((workspace) => workspace.id))
        : new Set(availableEntries(rest, recentEntries()).map((entry) => entry.key));
  const kept = reconcileCycle(cycle.items, cycle.index, (item) => alive.has(item.key));
  if (!kept) return null;
  // An Agent pane cycle whose origin is not, or is no longer, a row may hold one: that row is still somewhere to go.
  const fewest = cycle.kind === "agents" && !kept.items.some((item) => item.key === cycle.originKey) ? 1 : 2;
  return kept.items.length >= fewest ? { ...cycle, ...kept } : null;
}

/** What ⌘n, ⌥n or ⌥⌘n selects now, or null when nothing holds that number. */
export function numberedTarget(family: NumberedFamily, number: Digit, state: { rest: SnapshotRest | null; agents: AgentRow[] }): string | null {
  if (family === "tabs") {
    const frame = areaFrame("agent");
    return frame ? numberedTabs(frame.layout).get(number) ?? null : null;
  }
  if (family === "devices") return numberedDevices(state.rest?.navigator?.devices).get(number) ?? null;
  return sidebarNumbers(state, useUiStore.getState()).get(number) ?? null;
}

/**
 * Releasing the held modifier: one move to the row the operator stopped on,
 * none when they stopped where they started. A Workspace surface comes
 * forward in its own checkout and the Workspace shows once it is in front
 * (`openSurface`); All projects or an Overview is the page's own screen and
 * shows at once. A project with no surface to restore comes forward as its
 * checkout does from the sidebar, which is also how a device's project comes
 * forward.
 */
/** Commits the chosen item; false when nothing moved: the cycle chose its origin, or a target that left its scope. */
export function commitCycle(cycle: Cycle, actions: Actions): boolean {
  const chosen = cycle.items[cycle.index];
  const atOrigin = cycle.kind === "panels" || cycle.kind === "projects" ? cycle.index === 0 : chosen?.key === cycle.originKey;
  if (!chosen || atOrigin) return false;
  if (cycle.scope) {
    const membership = scopedSurfaces(useShellStore.getState().rest, cycle.scope);
    if (chosen.target.kind !== "surface" || !membership?.surfaces.some((surface) => surface.key === chosen.key)) return false;
    actions.focusView(chosen.target.surface.id, true);
    return true;
  }
  const target = chosen.target;
  switch (target.kind) {
    case "surface":
      actions.openSurface(target.surface);
      return true;
    case "pane":
      // The tab and the pane both wait for the keyboard to land, so the frames on the way are not visits.
      expectSurface(target.surface.key);
      expectPane(target.paneId);
      // As an agent chosen from the Agents list: its Workspace, device and pane in one event.
      actions.openAgent(target.paneId);
      return true;
    case "screen":
      // Expected like any commit, so an earlier commit still on its way cannot keep this from being the visit.
      expectSurface(chosen.key);
      // The rail and the sidebar follow the device first; the page's screen shows at once.
      actions.focusDevice(target.deviceId);
      useUiStore.getState().restoreScreen(target.screen);
      return true;
    case "checkout":
      actions.openWorkspace(target.deviceId, target.workspaceId, target.checkoutId);
      return true;
  }
}

/**
 * The close chords (⌘W and ⇧⌘W, Ctrl+Shift+W and Alt+Shift+W off macOS, and
 * a browser's ⌥ forms) close what covers the screen before anything under
 * it, as a window's close does on every system: the innermost menu, popover,
 * dialog or sheet (Settings, the Overview over a Workspace), a close's
 * confirmation (kept open, as Escape keeps it), an overlay, or the Overview
 * screen. The pane or view under a cover is out of sight, so a chord never
 * reaches it. An inline layer (a filter, the pane's find bar) covers nothing
 * and the chord passes it. Returns whether the chord was spent here.
 */
export function closeCovering(actions: Pick<Actions, "keepOpen" | "closeOverview">): boolean {
  const state = useUiStore.getState();
  const layer = coveringLayer(state.escapeLayers);
  if (layer) {
    layer();
    return true;
  }
  if (state.pendingClose) {
    actions.keepOpen();
    return true;
  }
  if (state.overlay !== "none" && state.overlay !== "find") {
    state.closeOverlay();
    return true;
  }
  if (state.screen?.kind === "main") {
    actions.closeOverview();
    return true;
  }
  return false;
}

function advance(cycle: Cycle, backward: boolean): Cycle {
  const count = cycle.items.length;
  if (cycle.index < 0) return { ...cycle, index: backward ? count - 1 : 0 };
  return { ...cycle, index: (cycle.index + (backward ? -1 : 1) + count) % count };
}

export function installKeyboard(actions: Actions): () => void {
  const ui = () => useUiStore.getState();
  const host = hostKind();
  const system = keySystem();
  const removeKeyboardOwner = installKeyboardOwner();
  const unsubscribeOwner = subscribeKeyboardOwner(() => {
    const rest = useShellStore.getState().rest;
    const owner = keyboardCommandOwner();
    if (focusedCycleScope(rest, owner) || agentOrigin(rest, owner)) observeRecent(rest, true);
  });
  // The modifier whose release commits the running cycle: ⌥ for the ⌥ family,
  // ⌃ for the desktop app's ⌃Tab.
  let cycleRelease = "Alt";
  let nativeCycle: { cycleId: number; workspace: string; id: string } | null = null;
  const endNativeCycle = () => {
    if (!nativeCycle) return;
    browserBridge()?.endCycle(nativeCycle.cycleId);
    nativeCycle = null;
  };

  const run = (id: CommandId, event: KeyboardEvent | null) => {
    // While a Herdr on another protocol holds Hide, no command reaches a workspace (PRD herdr-takeover).
    if (herdrGate(useShellStore.getState().rest)) return;
    if (isNumberedCommand(id)) {
      // An empty number is nothing, not a diagnostic: the hold hint shows
      // which numbers exist, and pressing past them is an ordinary miss.
      // The guard narrows `id` for the switch below; the second lookup is the family and number.
      const numbered = numberedCommand(id)!;
      const target = numberedTarget(numbered.family, numbered.number, useShellStore.getState());
      if (!target) return;
      if (numbered.family === "tabs") actions.selectAgentTab(target);
      else if (numbered.family === "devices") actions.focusDevice(target);
      else actions.openListedAgent(target);
      return;
    }
    switch (id) {
      case "new_tab":
        return actions.newTabFocused();
      case "close_tab":
        if (closeCovering(actions)) return;
        return actions.closeFocused();
      case "reopen_closed_tab":
        return actions.reopenClosed();
      case "new_workspace":
        return actions.openAddProject();
      case "start_agent":
        return actions.openStartPanel();
      case "recent_area_tab":
      case "previous_recent_area_tab":
      case "recent_panel":
      case "previous_recent_panel":
      case "recent_project":
      case "previous_recent_project": {
        const backward = id.startsWith("previous");
        const rest = useShellStore.getState().rest;
        const family = id.endsWith("area_tab") ? "area" : id.endsWith("panel") ? "panels" : "projects";
        const current = ui().cycle;
        const held = current && (current.kind === "agents" ? "area" : current.kind) === family;
        if (event && current && !held) return;
        const owner = keyboardCommandOwner();
        // The Agent area and a View area never both hold the keyboard, so at most one of these is a cycle.
        const fresh = () => family === "area" ? (agentCycle(rest, owner) ?? areaCycle(rest, owner)) : family === "panels" ? panelCycle(rest) : projectCycle(rest);
        const cycle = event && held ? reconcileHeldCycle(current, rest) : fresh();
        if (!cycle) return;
        const next = advance(cycle, backward);
        if (!event) { endNativeCycle(); ui().setCycle(null); commitCycle(next, actions); return; }
        const command = hostRegistry(rest?.ui_state, host, system).registry.find((row) => row.id === id);
        const chord = command && hostChord(command, host);
        const release = chord && releaseModifier(chord);
        if (!release) return;
        if (!current) cycleRelease = release;
        ui().setCycle(next);
        return;
      }
      case "search":
        return actions.openSearch();
      case "open_file":
        return actions.openFilePalette();
      case "toggle_left_sidebar":
        return actions.toggleLeftSidebar();
      case "overview":
        return actions.toggleOverview();
      case "sidebar_projects":
        return actions.focusSidebar();
      case "add_backlog":
        return actions.openQuickAdd();
      case "toggle_device_rail":
        return actions.toggleDeviceRail();
      case "toggle_lane":
        return actions.toggleLane();
      case "toggle_sidebar_lens":
        return actions.toggleSidebarLens();
      case "toggle_lane_project":
        return actions.toggleTool("project");
      case "toggle_lane_memory":
        return actions.toggleTool("memory");
      case "toggle_lane_explorer":
        return actions.toggleTool("explorer");
      case "toggle_lane_changes":
        return actions.toggleTool("changes");
      case "open_project_memory":
        return actions.openFrontProjectMemory();
      case "open_sidebar_prs":
        return actions.openSidebarPrs();
      case "toggle_lane_documents":
        return actions.toggleDocuments();
      case "find_in_pane": {
        // One chord, two surfaces, chosen by where the operator works: the
        // View area they are in finds in its document, anywhere else the
        // focused terminal pane's find bar opens, even with an editor open
        // beside it.
        const { editor } = useShellStore.getState();
        if (editorFor(editor) && drawnViews() && keyboardCommandOwner().kind === "view") return actions.requestEditorFind();
        return actions.openFind();
      }
      case "save_file":
        return actions.saveFile();
      case "keep_open":
        // The same chord answers a pending close and the active preview
        // display: the confirmation is showing first, so it wins when it is.
        if (ui().pendingClose) return actions.keepOpen();
        return actions.keepViewOpen();
      case "split_right":
        return actions.split("right");
      case "split_down":
        return actions.split("down");
      case "toggle_zoom": {
        // One chord for "this, full size", chosen by where the operator
        // works: in the right lane it expands the lane and back, anywhere
        // else it zooms the focused terminal pane.
        const owner = keyboardCommandOwner();
        const lane = ui().drawnLane.mode;
        if ((owner.kind === "view" || owner.kind === "tool" || owner.kind === "detail") && lane !== "closed") return ui().setLaneExpanded(!ui().laneExpanded);
        return actions.toggleZoom();
      }
      case "toggle_plan":
        return actions.togglePlanChip();
      case "close_pane":
        if (closeCovering(actions)) return;
        return actions.closePane();
      case "text_larger":
        return actions.textScale("in");
      case "text_smaller":
        return actions.textScale("out");
      case "text_reset":
        return actions.textScale("reset");
      case "focus_next_agent_area":
        return actions.runAgentCommand("focus_next");
      case "focus_previous_agent_area":
        return actions.runAgentCommand("focus_previous");
      case "grow_agent_area":
        return actions.runAgentCommand("grow");
      case "shrink_agent_area":
        return actions.runAgentCommand("shrink");
      case "focus_next_view_area":
        return actions.runViewArea("focus_next");
      case "focus_previous_view_area":
        return actions.runViewArea("focus_previous");
      case "grow_view_area":
        return actions.runViewArea("grow");
      case "shrink_view_area":
        return actions.runViewArea("shrink");
      case "shortcuts":
        return actions.openShortcuts();
      case "settings":
        return actions.openSettings();
      case "move_to_trash":
      case "copy_path":
      case "copy_relative_path":
      case "explorer_cut":
      case "explorer_copy":
      case "explorer_paste":
        // The Explorer's own chords: the tree answers them while it holds the keyboard.
        return;
      default: {
        const never: never = id;
        useShellStore.getState().noteDiagnostic(`shortcut without an action: ${String(never)} (${event?.code ?? "menu"})`);
      }
    }
  };

  // The hold hint. `hint` is the pure state; `timer` wakes `advance` at its
  // deadline; `publish` hands the ui store the family the state reveals and
  // nothing else, so a hold that never reveals costs no render.
  let hint: HintState = idleHint();
  let timer: ReturnType<typeof setTimeout> | null = null;
  const publish = () => {
    const { registry } = hostRegistry(useShellStore.getState().rest?.ui_state, host, system);
    ui().setHint(revealedFamily(hint, registry, host));
  };
  const setHint = (next: HintState) => {
    const changed = next !== hint;
    // A timer may wake before the fractional deadline. Keep one wake pending
    // without publishing until the pure state actually advances.
    if (!changed && (timer !== null || hint.deadline === null)) return;
    hint = next;
    if (timer) {
      clearTimeout(timer);
      timer = null;
    }
    if (hint.deadline !== null) {
      timer = setTimeout(() => {
        timer = null;
        setHint(advanceHint(hint, performance.now()));
      }, Math.max(1, Math.ceil(hint.deadline - performance.now())));
    }
    if (changed) publish();
  };
  // A blur or a layer with no hold in progress changes nothing and publishes nothing.
  const endHold = () => {
    if (hint.deadline !== null || hint.revealed || hint.modifiers !== NO_MODIFIERS) setHint(clearHint());
  };
  const MODIFIER_KEYS = new Set(["Meta", "Alt", "Shift", "Control"]);
  const onVisibility = () => {
    if (document.visibilityState === "hidden") endHold();
  };
  // A layer opening over the page ends the hold (B6): the keycaps name
  // what a chord would select, and a sheet, a menu, a dialog or a cycle
  // is what the keyboard now belongs to.
  const layerOpen = (state: ReturnType<typeof ui>) =>
    state.overlay !== "none" || state.escapeLayers.length > 0 || state.workspaceDialog !== null || state.pendingClose !== null || state.pendingTrash !== null || state.cycle !== null;
  const unsubscribeLayers = useUiStore.subscribe((state, previous) => {
    if (layerOpen(state) && !layerOpen(previous)) endHold();
    // Release, Escape and blur settle a native hold themselves. One that
    // ends any other way (its scope shrank to one tab, the daemon went away)
    // moved nothing, so the page that started it takes the keyboard back.
    if (previous.cycle && !state.cycle && nativeCycle) {
      const originalPage = nativeCycle;
      endNativeCycle();
      focusBrowserDisplay(originalPage.workspace, originalPage.id);
    }
  });

  const onKeyDown = (event: KeyboardEvent) => {
    if (MODIFIER_KEYS.has(event.key)) {
      if (!layerOpen(ui())) setHint(holdModifiers(hint, modifiersOf(event), performance.now()));
    } else if (hint.deadline !== null || hint.revealed) {
      // A key during the hold is the chord itself (⌘2, ⌘C): the hint ends
      // now and comes back only with a fresh hold (B6, B8).
      endHold();
    }
    if (event.isComposing || event.keyCode === 229) {
      // An Escape while an input method composes belongs to the input method,
      // which has already answered it; Radix's own Escape listener does not
      // ask, and would close the dialog the operator is typing Hangul in. A
      // terminal's textarea keeps it: xterm decides what a composing key does.
      const terminal = event.target instanceof Element && event.target.closest(".xterm");
      if (event.key === "Escape" && !terminal) event.stopPropagation();
      return;
    }
    // AltGr types a character on Windows and Linux layouts, and Windows
    // reports it as Ctrl+Alt: a Polish Ń is AltGr+Shift+N, which would
    // otherwise read as a Ctrl+Alt+Shift chord.
    if (system === "pc" && event.getModifierState?.("AltGraph")) return;
    // A Shortcuts row that is recording owns the next chord, Escape included:
    // no command runs while the operator is showing the recorder a key.
    if (ui().recordingShortcut) return;
    if (event.key === "Escape") {
      // An Escape the shell answers is consumed here: the pane's textarea is
      // still the focused element under the sheet or a cycle, and xterm
      // would send the same press to the program as an ESC byte. A tooltip
      // open on hover is not a layer and closes through its own dismiss,
      // which never sees a consumed press, so consuming closes it here.
      const consume = () => {
        event.preventDefault();
        event.stopPropagation();
        for (const close of ui().tooltips) close();
      };
      if (ui().cycle) {
        const originalPage = nativeCycle;
        endNativeCycle();
        ui().setCycle(null);
        if (originalPage) focusBrowserDisplay(originalPage.workspace, originalPage.id);
        consume();
        return;
      }
      const innermost = ui().escapeLayers.at(-1);
      if (innermost) {
        innermost();
        consume();
        return;
      }
      if (ui().pendingClose) {
        actions.keepOpen();
        consume();
        return;
      }
      if (ui().overlay !== "none") {
        ui().closeOverlay();
        consume();
        return;
      }
      // With no layer open, Escape leaves a Project's Overview for the
      // Workspace in front, or All projects when there is none
      // (web-project-overview B1). Every dialog, menu and popover is an escape
      // layer, answered above, and a tooltip is closed by `consume`; a text field
      // holding text answers it itself, so the Sessions search clears before a
      // second Escape leaves.
      const field = event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement ? event.target : null;
      if (ui().screen?.kind === "main" && !field?.value) {
        actions.closeOverview();
        consume();
      }
      return;
    }
    const { registry } = hostRegistry(useShellStore.getState().rest?.ui_state, host, system);
    const command = matchHost(event, registry, host);
    if (!command) return;
    event.preventDefault();
    event.stopPropagation();
    run(command.id, event);
  };

  // Releasing the held modifier commits the cycle (`commitCycle`).
  const onKeyUp = (event: KeyboardEvent) => {
    if (MODIFIER_KEYS.has(event.key)) setHint(holdModifiers(hint, modifiersOf(event), performance.now()));
    if (event.key !== cycleRelease) return;
    const cycle = ui().cycle;
    if (!cycle) return;
    const originalPage = nativeCycle;
    endNativeCycle();
    ui().setCycle(null);
    // A release that moved nothing returns the keyboard to the page that
    // started the hold; a commit hands it to the chosen destination instead.
    if (!commitCycle(cycle, actions) && originalPage) focusBrowserDisplay(originalPage.workspace, originalPage.id);
  };

  // Losing the window mid-cycle (⌥-Tab switching apps) cancels it; nothing
  // is committed for a chord the operator did not finish here.
  const onBlur = () => {
    // The host gives the page back its keyboard when the window returns.
    endNativeCycle();
    if (ui().cycle) ui().setCycle(null);
    endHold();
  };

  // A menu item names a command id; one this registry does not know is a
  // host/shell version mismatch, recorded rather than guessed at.
  const bridge = hostBridge();
  const unsubscribeBrowser = browserBridge()?.onEvent((input) => {
    if (input.kind === "cycle-cancel") {
      if (nativeCycle?.cycleId === input.cycleId) onBlur();
      return;
    }
    if (input.kind !== "cycle-input") return;
    const firstStart = !nativeCycle && !ui().cycle && input.type === "keyDown";
    nativeCycle = { cycleId: input.cycleId, workspace: input.workspace, id: input.id };
    const cycle = ui().cycle;
    const scope = focusedCycleScope(useShellStore.getState().rest);
    const frame = drawnViews();
    if (!cycle && (!scope || scope.kind !== "view" || !frame || `${frame.workspace.device_id}\u0000${frame.workspace.path}` !== input.workspace || focusedSurface(useShellStore.getState().rest, scope)?.id !== input.id)) {
      if (firstStart) focusBrowserDisplay(input.workspace, input.id);
      endNativeCycle();
      return;
    }
    const event = new KeyboardEvent(input.type === "keyDown" ? "keydown" : "keyup", { code: input.code, key: input.key, ctrlKey: input.control, altKey: input.alt, metaKey: input.meta, shiftKey: input.shift });
    if (input.type === "keyDown") onKeyDown(event); else onKeyUp(event);
    if (!ui().cycle) {
      // A rejected or zero/one-item start borrowed the shell's responder,
      // but selected nothing. A completed cycle follows its chosen page instead.
      if (firstStart) focusBrowserDisplay(input.workspace, input.id);
      endNativeCycle();
    }
  });
  const unsubscribeMenu = bridge?.onCommand((id) => {
    // The screen that holds Hide answers no menu item (PRD herdr-takeover).
    if (herdrGate(useShellStore.getState().rest)) return;
    // Help > Set up Hide, the host's own item outside the registry (PRD first-run-onboarding B45).
    if (id === SETUP_COMMAND) {
      useSetupWalk.getState().open();
      return;
    }
    // Check for Updates…: the host has started the check; its outcome shows in About (PRD auto-update D-20).
    if (id === CHECK_UPDATES_COMMAND) {
      ui().openSettings("about");
      return;
    }
    const command = REGISTRY.find((row) => row.id === id);
    if (command) {
      run(command.id, null);
      noteCommandDelivered();
    } else useShellStore.getState().noteDiagnostic(`menu command the shell does not know: ${id}`);
  });
  // The app menu's accelerators follow the stored macOS set: the host gets
  // it once now and again whenever its contents change, and resolves it with
  // the same rules this listener runs.
  // The store notifies per snapshot; an unchanged set is the same object, so
  // most notifications end at the reference check.
  let seen: Record<string, string> | undefined | null = null;
  let reported: string | null = null;
  const report = () => {
    const stored = storedBindings(useShellStore.getState().rest?.ui_state, "electron");
    if (stored === seen) return;
    seen = stored;
    const text = JSON.stringify(stored ?? {});
    if (text === reported) return;
    reported = text;
    bridge?.reportBindings(stored ?? {});
  };
  // The native menu and status page follow the core's explicit language
  // choice, which only a confirmed snapshot carries; null means the system's.
  let languageReported: string | null | undefined;
  const reportLanguage = () => {
    const rest = useShellStore.getState().rest;
    if (!rest) return;
    const choice = rest.ui_state?.interface_language ?? null;
    if (choice === languageReported) return;
    languageReported = choice;
    bridge?.reportLanguage(choice);
  };
  const unsubscribeBindings = bridge ? useShellStore.subscribe(() => {
    report();
    reportLanguage();
  }) : null;
  if (bridge) {
    report();
    reportLanguage();
  }

  window.addEventListener("keydown", onKeyDown, true);
  window.addEventListener("keyup", onKeyUp, true);
  window.addEventListener("blur", onBlur);
  document.addEventListener("visibilitychange", onVisibility);
  return () => {
    onBlur();
    endNativeCycle();
    removeKeyboardOwner();
    unsubscribeBrowser?.();
    unsubscribeOwner();
    unsubscribeMenu?.();
    unsubscribeBindings?.();
    unsubscribeLayers();
    if (timer) clearTimeout(timer);
    ui().setHint(null);
    window.removeEventListener("keydown", onKeyDown, true);
    window.removeEventListener("keyup", onKeyUp, true);
    window.removeEventListener("blur", onBlur);
    document.removeEventListener("visibilitychange", onVisibility);
  };
}
