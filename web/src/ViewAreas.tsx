import { GlobeIcon, Link2Icon, LoaderCircleIcon, XIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Actions } from "./actions";
import { createAreaTree, tabFit, type AreaAdapter, type AreaTabInteraction } from "./AreaTree";
import { AreaEmpty } from "./AreaEmpty";
import { BrowserDisplay } from "./BrowserDisplay";
import { focusBrowserDisplay, hostKey, useBrowserStore } from "./browserViews";
import { Badge } from "./components/ui/badge";
import { FileMark } from "./components/file-mark";
import { Button } from "./components/ui/button";
import { Hint } from "./components/ui/tooltip";
import { revealHost } from "./host";
import { CommentCount, FeedbackRow, OpenerMark } from "./ViewComments";
import { DisplayEditor, DocumentKeeper } from "./Editor";
import { fileIcon } from "./fileIcons";
import { useInterfaceTranslation } from "./i18n/client";
import { windowDeviceId } from "./screenMachine";
import { editorTabFor, frontCheckout, type ViewDisplaySnapshot, type ViewLayoutSnapshot } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { commandLabel } from "./shortcutLabels";
import { noteDrawnViews, useKeyboardOwner } from "./viewFocus";
import { displayIdentity, displayMenu, focusRequestArrived, placeKey, shownDisplays, workspaceKey, showsSameDocument, VIEW_WORDS, type ViewMenuId, type ViewWorkspace } from "./viewLayout";
import { locateDisplay } from "./areaLayout";
import { workspaceViewOf } from "./workspace";

/**
 * The front Workspace's View areas, the lane's files tool. With no display
 * the files tool draws its tree alone (`laneFrame`'s `treeOnly`), so these
 * are drawn only while a view is open or a file of this checkout is opening.
 */
export function ViewAreas({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const view = useShellStore((s) => workspaceViewOf(s.rest));
  if (!view) return null;
  const layout = view.layout;
  if (!layout) {
    return <AreaEmpty state="view-layout-missing" text={t("documents.view.noLayout")} />;
  }
  if (layout.display_count === 0) {
    return <AreaEmpty state="view-opening" text={t("documents.view.opening")} />;
  }
  // One tree per Workspace: a front that moves to another Workspace ends a
  // drag, a divider drag or a menu begun on this one, whose ids (a1, d2, s1)
  // name other views there (contract 4.1, B8).
  return <ViewTree key={workspaceKey({ device_id: view.device_id, path: view.path })} layout={layout} deviceId={view.device_id} path={view.path} actions={actions} />;
}

const SharedViewTree = createAreaTree<ViewDisplaySnapshot>("view");
/**
 * An editor still on its way: a document or diff whose text is being read
 * (`Notice` in `Editor.tsx`), or a host whose editor its effect has yet to build.
 */
const READING = [
  '[data-editor-state="loading"]',
  '[data-editor-state="diff-loading"]',
  "[data-editor-codemirror]:not(:has(.cm-content))",
  "[data-patch-view]:not(:has(.cm-content))",
].join(", ");
function ViewTree({ layout, deviceId, path, actions }: { layout: ViewLayoutSnapshot; deviceId: string; path: string; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const owner = useKeyboardOwner();
  const checkoutId = useShellStore((s) => frontCheckout(s.rest)?.id ?? null);
  const workspace = useMemo(() => ({ device_id: deviceId, path }), [deviceId, path]);
  const key = workspaceKey(workspace);
  const [body, setBody] = useState<HTMLDivElement | null>(null);
  // A menu, the palette, a drop or an open moved the keyboard's place; once
  // the core shows it there, the keyboard follows, and this focus asks the core for
  // nothing (B20). A move of the active view resolves on the frame that
  // lands it, never on the one it was asked from. The keyboard goes there
  // once that frame's editors have settled, since an editor that moved to
  // a new area is built as it mounts (twice, in a development build), and
  // not when something else took the keyboard in between.
  // A file or diff still being read has no editor yet: its tab holds the
  // keyboard, and the editor takes it as it mounts unless the operator has
  // moved it by then. One watch per tree, ended when the read settles, by
  // the next arrival, or with the tree.
  const request = useUiStore((s) => s.viewFocusRequest);
  const opens = useShellStore((s) => s.rest?.status?.browser_opens);
  const contentWatch = useRef<MutationObserver | null>(null);
  useEffect(() => () => contentWatch.current?.disconnect(), []);
  useEffect(() => {
    if (!request || !body || !focusRequestArrived(request, key, layout, opens)) return;
    useUiStore.getState().setViewFocusRequest(null);
    const areaId = layout.active_area;
    const before = document.activeElement;
    requestAnimationFrame(() => {
      contentWatch.current?.disconnect();
      contentWatch.current = null;
      const now = document.activeElement;
      if (now !== before && now !== null && now !== document.body) return;
      const area = body.querySelector<HTMLElement>(`[data-view-area-id="${CSS.escape(areaId)}"]`);
      if (!area) return;
      const browserId = area.querySelector<HTMLElement>("[data-browser-slot]")?.dataset.browserSlot;
      if (browserId) {
        focusBrowserDisplay(key, browserId);
        return;
      }
      const content = area.querySelector<HTMLElement>(".cm-content");
      const tab = area.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]');
      (content ?? tab)?.focus({ preventScroll: true });
      if (content || !tab || !area.querySelector(READING)) return;
      const watch = new MutationObserver(() => {
        const mounted = area.querySelector<HTMLElement>(".cm-content");
        if (document.activeElement !== tab || (!mounted && !area.querySelector(READING))) watch.disconnect();
        else if (mounted) {
          watch.disconnect();
          mounted.focus({ preventScroll: true });
        }
      });
      watch.observe(area, { childList: true, subtree: true });
      contentWatch.current = watch;
    });
  }, [request, key, layout, body, opens]);

  // One keeper per document on screen, however many displays show it.
  const documents = useMemo(
    () => [...new Set(shownDisplays(layout.root).flatMap((display) => (display.state === "open" && display.kind === "file" && display.tab_id ? [display.tab_id] : [])))],
    [layout.root],
  );


  const adapter: AreaAdapter<ViewDisplaySnapshot> = {
    words: VIEW_WORDS,
    keyboardArea: owner.kind === "view" && owner.workspace === checkoutId ? owner.areaId : null,
    label: (display) => display.label,
    sameContent: showsSameDocument,
    tab: (display, interaction) => <DisplayTab display={display} workspace={workspace} interaction={interaction} actions={actions} />,
    body: (display) => (
      <div key={display.id} className="flex min-h-0 min-w-0 flex-1 flex-col">
        <DisplayBody display={display} workspace={workspace} actions={actions} />
        <FeedbackRow display={display} workspace={workspace} actions={actions} />
      </div>
    ),
    empty: () => <AreaEmpty state="no-view" text={t("documents.view.empty")} />,
    floating: (display) => <>{displayMark(display)}<span className={`truncate ${display.preview ? "italic" : ""}`}>{display.label}</span></>,
    menu: (id, geometry, sizes) => [{ id: "new_tab", label: t("panes.area.newTab"), unavailable: null }, ...displayMenu(layout, geometry, sizes, id, { host: revealHost(), device: workspace.device_id, node: windowDeviceId(useShellStore.getState().rest?.navigator?.devices) })],
    runMenu: (id, displayId) => id === "new_tab" ? actions.openBrowser("", workspace, locateDisplay(layout.root, displayId)?.area.id) : actions.runViewMenu(id as ViewMenuId, displayId),
    focus: actions.focusView,
    focusArea: actions.focusViewArea,
    move: actions.moveView,
    split: actions.splitView,
    resize: actions.resizeViewSplit,
    newTab: (areaId) => actions.openBrowser("", workspace, areaId),
    newTabLabel: t("panes.area.newTab"),
    newTabShortcut: commandLabel("new_tab"),
    tabListLabel: t("documents.view.tabList"),
    actionsLabel: t("documents.view.actions"),
    onDraw: (frame) => noteDrawnViews(frame ? { ...frame, workspace } : null),
    onBody: setBody,
  };
  return <SharedViewTree layout={layout} adapter={adapter}>
    {documents.map((tabId) => <DocumentKeeper key={tabId} tabId={tabId} actions={actions} />)}
  </SharedViewTree>;
}

/** A display's body: its document or diff, or the state it is in (B16, contract 3). */
function DisplayBody({ display, workspace, actions }: { display: ViewDisplaySnapshot; workspace: ViewWorkspace; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  if (display.kind === "browser") return <BrowserDisplay display={display} workspace={workspace} actions={actions} />;
  if (display.state === "open") {
    return <DisplayEditor display={display} workspace={workspace} placeKey={placeKey(workspaceKey(workspace), display)} actions={actions} />;
  }
  if (display.state === "opening") return <AreaEmpty state="view-opening" text={t("documents.view.openingNamed", { label: display.label })} />;
  if (display.state === "waiting") return <AreaEmpty state="view-waiting" text={display.reason ?? t("documents.view.waitingPath", { path: display.path })} />;
  return (
    <AreaEmpty state="view-unavailable" text={display.reason ? t("documents.view.unavailableReason", { path: display.path, reason: display.reason }) : t("documents.view.unavailable", { path: display.path })}>
      {/* Each button is one action on this view alone, like a tab's ×: its
          press does not also make the area active (one action, one event). */}
      <Button variant="secondary" onPointerDown={(event) => event.stopPropagation()} onClick={() => actions.closeView(display.id)} data-close-unavailable={display.id}>
        {t("documents.view.closeView")}
      </Button>
      <Button variant="ghost" onPointerDown={(event) => event.stopPropagation()} onClick={() => actions.retryView(display.id)} data-retry-unavailable={display.id}>
        {t("common.retry")}
      </Button>
    </AreaEmpty>
  );
}

/** One display's tab: its kind's mark, italic while a preview, its save marks, and its whole identity (B2, B21). */
export function DisplayTab({ display, workspace, interaction, actions }: { display: ViewDisplaySnapshot; workspace?: ViewWorkspace; interaction: AreaTabInteraction; actions: Actions }) {
  const { selected, areaActive } = interaction;
  const fit = tabFit(selected, interaction.fit);
  const { t } = useInterfaceTranslation();
  const dirty = useShellStore((s) => editorTabFor(s.editor, display.tab_id)?.dirty ?? false);
  const saving = useShellStore((s) => display.tab_id !== null && s.savingTabs.has(display.tab_id));
  const tabOnly = useShellStore((s) => display.tab_id !== null && s.bufferWarnings.has(display.tab_id));
  const unavailable = display.state === "unavailable";
  const attached = useBrowserStore((s) => display.kind === "browser" && workspace !== undefined && s.attached[hostKey(workspaceKey(workspace), display.id)] === true);
  const state = saving ? t("documents.tabSaving") : dirty ? t("documents.unsaved") : null;
  const comments = display.comments ?? 0;
  const identity = [
    displayIdentity(display, t),
    state,
    tabOnly ? t("documents.tabOnly") : null,
    attached ? t("documents.agentAttached") : null,
    comments > 0 ? t("feedback.tab.comments", { count: comments }) : null,
    (display.not_delivered ?? 0) > 0 ? t("feedback.tab.notDelivered") : null,
  ].filter(Boolean).join(" · ");
  const closeLabel = t("documents.closeView", { label: display.label });
  return (
    <Hint label={identity} reveals>
    <div
      role="tab"
      aria-selected={selected}
      aria-label={identity}
      tabIndex={0}
      data-tab={display.tab_id ?? ""}
      data-display={display.id}
      data-tab-kind={display.kind}
      data-preview={display.preview ? "true" : "false"}
      data-saving={saving ? "true" : "false"}
      data-tab-only={tabOnly ? "true" : "false"}
      data-unavailable={unavailable ? "true" : "false"}
      data-view-state={display.state}
      className={`group relative flex min-w-0 flex-1 cursor-default select-none items-center gap-xs ${fit.tab} text-caption outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring ${
        selected ? `text-foreground ${areaActive ? "bg-background" : "bg-secondary"}` : "text-subtle-foreground hover:bg-accent"
      } ${interaction.dragging ? "opacity-[var(--opacity-dimmed)]" : ""}`}
      onPointerDown={interaction.press}
      onClick={() => {
        interaction.select();
      }}
      onDoubleClick={() => actions.keepViewOpen(display.id)}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          interaction.select();
        }
      }}
    >
      <span className={`relative flex size-(--size-icon) shrink-0 items-center justify-center ${unavailable ? "text-muted-foreground opacity-[var(--opacity-dimmed)]" : ""}`}>
        {interaction.fit === "marks" ? collapsedDisplayMark(display) : displayMark(display)}
        {interaction.fit === "marks" && (saving || dirty || tabOnly) ? (
          saving ? <LoaderCircleIcon aria-hidden="true" data-view-save-mark="saving" className="absolute size-(--size-icon) animate-spin bg-inherit text-muted-foreground" />
            : <span aria-hidden="true" data-view-save-mark={tabOnly ? "tab-only" : "dirty"} className="absolute right-0 top-0 size-(--size-tab-status-dot) rounded-full bg-warning" />
        ) : null}
      </span>
      {attached ? <Badge aria-hidden="true" data-browser-attached="true"><Link2Icon /></Badge> : null}
      <span className={`min-w-0 flex-1 truncate ${fit.title} ${display.preview ? "italic" : ""} ${unavailable ? "text-muted-foreground line-through" : ""}`}>
        {display.label}
        {saving ? <span className="text-muted-foreground"> {t("documents.tabSaving")}</span> : dirty ? <span className="text-warning"> ●</span> : null}
        {tabOnly ? <span className="text-muted-foreground"> {t("documents.tabOnly")}</span> : null}
      </span>
      {display.opener ? <OpenerMark opener={display.opener} actions={actions} /> : null}
      <CommentCount display={display} />
      <Hint label={closeLabel}>
        <Button
          variant="ghost"
          size="icon-sm"
          className={`${fit.close} shrink-0 hover:bg-popover hover:text-foreground focus-visible:visible group-hover:visible ${selected ? "visible" : "invisible"}`}
          aria-label={closeLabel}
          onPointerDown={(event) => event.stopPropagation()}
          onClick={(event) => {
            event.stopPropagation();
            actions.closeView(display.id);
          }}
        >
          <XIcon />
        </Button>
      </Hint>
      {selected && areaActive ? <span className="absolute inset-x-0 bottom-0 h-[var(--size-tab-indicator)] bg-foreground" /> : null}
    </div>
    </Hint>
  );
}

/** A file's type mark or the diff's comparison mark; never colour alone (D-15). */
export function displayMark(display: Pick<ViewDisplaySnapshot, "kind" | "label">) {
  if (display.kind === "browser") return <GlobeIcon aria-hidden="true" data-view-mark="browser" className="size-(--size-icon) shrink-0 text-muted-foreground" />;
  if (display.kind === "diff") {
    return (
      <span aria-hidden="true" data-view-mark="diff" className="shrink-0 font-mono text-warning">
        ±
      </span>
    );
  }
  return <FileMark icon={fileIcon(display.label)} data-view-mark="file" />;
}

/** The minimum-density identity choice lives here so review can replace it without changing the fit curve. */
function collapsedDisplayMark(display: Pick<ViewDisplaySnapshot, "kind" | "label">) {
  return displayMark(display);
}
