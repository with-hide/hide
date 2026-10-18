import { ArrowLeftIcon, ArrowRightIcon, MinusIcon, MonitorIcon, MonitorSmartphoneIcon, PlusIcon, RotateCwIcon, ScanIcon, SmartphoneIcon, TabletIcon, XIcon, type LucideIcon } from "lucide-react";
import { Fragment, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import type { Actions } from "./actions";
import { AreaEmpty } from "./AreaEmpty";
import { addressShown, addressUrl, clearBrowserAttachments, focusBrowserDisplay, hostKey, mountBrowserAttachments, noteBrowserAttachment, notePageState, parseWorkspaceKey, registerBrowserSlot, syncBrowserFront, useBrowserStore } from "./browserViews";
import { NewTabBody } from "./components/new-tab-body";
import { changedFiles } from "./newTab";
import { useUiStore } from "./ui";
import { Button } from "./components/ui/button";
import { Input } from "./components/ui/input";
import { badgeVariants } from "./components/ui/badge";
import { DropdownMenu, DropdownMenuCheckboxItem, DropdownMenuContent, DropdownMenuSeparator, DropdownMenuTrigger } from "./components/ui/dropdown-menu";
import { Popover, PopoverContent, PopoverTrigger } from "./components/ui/popover";
import { Hint } from "./components/ui/tooltip";
import { BROWSER_DEVICES, deviceLabel, devicePlacement, type BrowserDevice } from "./browserDevice";
import { cn } from "./lib/utils";
import { browserBridge, type BrowserCommand } from "./host";
import type { ViewDisplaySnapshot } from "./snapshot";
import { useShellStore } from "./store";
import { locateDisplay, workspaceKey, type ViewWorkspace } from "./viewLayout";
import { workspaceViewOf } from "./workspace";
import { commandLabel } from "./shortcutLabels";
import { TreeButton } from "./TreeButton";
import { useInterfaceTranslation } from "./i18n/client";
import { MessageSquarePlusIcon } from "lucide-react";
import { browserCommentKey, commentsOn, useFeedbackStore, noteBrowserComment } from "./viewFeedback";
import { PageNoteLayer, useViewFeedback } from "./ViewComments";

// A browser display (issue 155): its toolbar and the place its page shows.
// In the desktop app the page is a native view the host lays over the slot
// below the toolbar; a plain browser tab cannot draw one there, and says so
// with a way to open the address in a tab of its own. Either way the toolbar
// row carries the address, so the row lines up with a document header beside
// it (issue 170).

export function BrowserDisplay({ display, workspace, actions }: { display: ViewDisplaySnapshot; workspace: ViewWorkspace; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const bridge = browserBridge();
  const url = display.url ?? "";
  if (!url) return <NewTab display={display} actions={actions} />;
  if (!bridge) {
    const web = /^https?:/i.test(url);
    return (
      <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-browser-display={display.id}>
        <div className="flex h-(--size-tab-strip) shrink-0 items-center gap-xs border-b border-border pl-xs pr-md" data-browser-toolbar={display.id}>
          <TreeButton actions={actions} />
          <span className="min-w-0 flex-1 truncate font-mono text-caption text-subtle-foreground" data-browser-address="true">
            {addressShown(url)}
          </span>
        </div>
        <AreaEmpty state="browser-host" text={t("documents.browserDesktopOnly")}>
          {web ? (
            <Button variant="secondary" data-browser-open-external={display.id} onClick={() => window.open(url, "_blank", "noopener,noreferrer")}>
              {t("documents.openInBrowser")}
            </Button>
          ) : null}
        </AreaEmpty>
      </div>
    );
  }
  return <HostedPage display={display} workspace={workspaceKey(workspace)} actions={actions} />;
}

function NewTab({ display, actions }: { display: ViewDisplaySnapshot; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const root = useShellStore((s) => s.rest?.navigator?.changes_root_path ?? null);
  const changes = useShellStore((s) => s.changes);
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-browser-display={display.id}>
      <div className="flex h-(--size-tab-strip) shrink-0 items-center gap-xs border-b border-border pl-xs pr-sm" data-browser-toolbar={display.id}>
        <TreeButton actions={actions} />
        <Hint label={t("common.back")}><Button variant="ghost" size="icon-sm" disabled><ArrowLeftIcon /></Button></Hint>
        <Hint label={t("documents.browserForward")}><Button variant="ghost" size="icon-sm" disabled><ArrowRightIcon /></Button></Hint>
        <Hint label={t("documents.browserReload")}><Button variant="ghost" size="icon-sm" disabled><RotateCwIcon /></Button></Hint>
        <AddressField address="" autoFocus onSubmit={(url) => actions.navigateBrowser(display.id, url)} />
      </div>
      <NewTabBody hasChanges={changedFiles(changes, root).length > 0} fileChord={commandLabel("open_file")} onFile={() => actions.openFilePalette()} onDiff={() => useUiStore.getState().openOverlay("diff_palette")} />
    </div>
  );
}

function HostedPage({ display, workspace, actions }: { display: ViewDisplaySnapshot; workspace: string; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const key = hostKey(workspace, display.id);
  const page = useBrowserStore((s) => s.pages[key] ?? null);
  const still = useBrowserStore((s) => s.stills[display.id]);
  const device = useUiStore((s) => s.browserDevices[key] ?? "fit");
  const size = devicePlacement(device);
  const slot = useRef<HTMLDivElement>(null);
  // The width the page's frame takes in the area, for the device button's scale.
  const [drawn, setDrawn] = useState(0);
  useLayoutEffect(() => {
    const element = slot.current;
    if (!element) return undefined;
    const placement = devicePlacement(device);
    const unregister = registerBrowserSlot(display.id, element, placement);
    if (!placement || typeof ResizeObserver === "undefined") return unregister;
    setDrawn(element.clientWidth);
    const observer = new ResizeObserver(() => setDrawn(element.clientWidth));
    observer.observe(element);
    return () => {
      observer.disconnect();
      unregister();
    };
  }, [display.id, device]);
  const command = (name: BrowserCommand) => browserBridge()?.command(workspace, display.id, name);
  const address = page?.url || display.url || "";
  const view = parseWorkspaceKey(workspace);
  const areaId = useAreaOf(display.id);
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-browser-display={display.id}>
      <div className="flex h-(--size-tab-strip) shrink-0 items-center gap-xs border-b border-border pl-xs pr-sm" data-browser-toolbar={display.id}>
        <TreeButton actions={actions} />
        <Hint label={t("common.back")}>
          <Button variant="ghost" size="icon-sm" disabled={!page?.canGoBack} onClick={() => command("back")} data-browser-command="back">
            <ArrowLeftIcon />
          </Button>
        </Hint>
        <Hint label={t("documents.browserForward")}>
          <Button variant="ghost" size="icon-sm" disabled={!page?.canGoForward} onClick={() => command("forward")} data-browser-command="forward">
            <ArrowRightIcon />
          </Button>
        </Hint>
        {page?.loading ? (
          <Hint label={t("documents.browserStop")}>
            <Button variant="ghost" size="icon-sm" onClick={() => command("stop")} data-browser-command="stop">
              <XIcon />
            </Button>
          </Hint>
        ) : (
          <Hint label={t("documents.browserReload")}>
            <Button variant="ghost" size="icon-sm" onClick={() => command("reload")} data-browser-command="reload">
              <RotateCwIcon />
            </Button>
          </Hint>
        )}
        <AddressField
          address={address}
          onSubmit={(url) => actions.navigateBrowser(display.id, url)}
          trailing={size || !zoomed(page?.zoom ?? 1) ? null : <ZoomBadge zoom={page?.zoom ?? 1} command={command} />}
        />
        <DeviceButton device={device} drawn={drawn} onChoose={(next) => useUiStore.getState().setBrowserDevice(key, next)} />
        <BrowserCommentButton display={display} workspace={workspace} areaId={areaId} address={address} actions={actions} />
      </div>
      <DeviceFrame width={size?.width ?? null}>
        <div ref={slot} className={cn("relative min-h-0 min-w-0", size ? "h-full max-w-full shadow-lg ring-1 ring-border" : "flex-1")} style={size ? { width: size.width } : undefined} data-browser-slot={display.id} data-browser-device={device}>
          {page?.failure ? (
            <AreaEmpty state="browser-failed" text={t("documents.browserFailed", { address, reason: page.failure })}>
              <Button variant="secondary" onClick={() => command("reload")} data-browser-retry={display.id}>
                {t("documents.browserReload")}
              </Button>
            </AreaEmpty>
          ) : still ? (
            <img src={still} alt="" aria-hidden="true" draggable={false} className="absolute inset-0 size-full select-none object-fill" data-browser-still={display.id} />
          ) : null}
          {view && areaId ? (
            <PageNoteLayer
              commentKey={browserCommentKey(view, display.id)}
              displayId={display.id}
              workspace={view}
              actions={actions}
              add={(pick, note) => actions.browserComment({ workspace: view, area_id: areaId, display_id: display.id, on: true, update: true, add: { pick, note } })}
              done={() => focusBrowserDisplay(workspace, display.id)}
            />
          ) : null}
        </div>
      </DeviceFrame>
    </div>
  );
}

/**
 * The area around a page shown at a device size (PRD browser-width-zoom
 * B12): the page's frame sits at the top, centered with the sides left empty
 * when it is narrower than the area, and filling the area's width when it is
 * wider, where the host scales the page down to fit it. A page that fits its
 * area fills it.
 */
function DeviceFrame({ width, children }: { width: number | null; children: ReactNode }) {
  // One element either way, so a size chosen keeps what the slot holds (a note being written).
  return (
    <div className={cn("flex min-h-0 min-w-0 flex-1", width === null ? "flex-col" : "justify-center bg-card px-md pt-md")} data-browser-device-area={width === null ? undefined : "true"}>
      {children}
    </div>
  );
}

/** A zoom factor as the badge and the popover show it. */
function zoomPercent(zoom: number): string {
  return `${Math.round(zoom * 100)}%`;
}

/** Chrome's zoom steps end at 25% and 500% (`zoomSteps.ts`). */
const ZOOM_LEAST = 0.25;
const ZOOM_MOST = 5;
const ZOOM_EPSILON = 0.001;

/** Whether a page's zoom is off 100%, which is when the badge shows. */
function zoomed(zoom: number): boolean {
  return Math.abs(zoom - 1) > ZOOM_EPSILON;
}

/**
 * The page's zoom inside the address field's right end, only while it is not
 * 100% (PRD browser-width-zoom B6-B9), so it mounts only then. It opens
 * − · zoom · + · Reset, which stays open while the operator steps, the
 * number following the page, and goes with the badge once the page is back
 * at 100%. The page keeps zooming on ⌘= / ⌘- /
 * ⌘0 as before; a press here leaves the keyboard in the shell.
 */
function ZoomBadge({ zoom, command }: { zoom: number; command: (name: BrowserCommand) => void }) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(false);
  const percent = zoomPercent(zoom);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <Hint label={t("documents.pageZoom")} shortcut={`${commandLabel("text_larger")} ${commandLabel("text_smaller")} ${commandLabel("text_reset")}`}>
        <PopoverTrigger asChild>
          <button
            type="button"
            className={cn(badgeVariants({ variant: "secondary" }), "cursor-pointer font-semibold text-foreground outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring")}
            aria-label={t("documents.pageZoomAt", { percent })}
            data-browser-zoom={percent}
          >
            {percent}
          </button>
        </PopoverTrigger>
      </Hint>
      <PopoverContent align="end" className="flex w-auto items-center gap-xxs p-xs" onOpenAutoFocus={(event) => event.preventDefault()} data-browser-zoom-popover="true">
        <Button variant="ghost" size="icon-sm" aria-label={t("documents.zoomOut")} disabled={zoom <= ZOOM_LEAST + ZOOM_EPSILON} onClick={() => command("zoom_out")} data-browser-zoom-step="out">
          <MinusIcon />
        </Button>
        <span className="min-w-(--size-control-lg) text-center text-caption font-semibold tabular-nums" aria-live="polite" data-browser-zoom-level="true">
          {percent}
        </span>
        <Button variant="ghost" size="icon-sm" aria-label={t("documents.zoomIn")} disabled={zoom >= ZOOM_MOST - ZOOM_EPSILON} onClick={() => command("zoom_in")} data-browser-zoom-step="in">
          <PlusIcon />
        </Button>
        <span aria-hidden="true" className="mx-xxs h-(--size-icon) w-(--size-hairline) bg-border" />
        <Hint label={t("documents.zoomResetHint")} shortcut={commandLabel("text_reset")}>
          <Button variant="ghost" size="sm" className="whitespace-nowrap" onClick={() => command("zoom_reset")} data-browser-zoom-step="reset">
            {t("documents.zoomReset")}
          </Button>
        </Hint>
      </PopoverContent>
    </Popover>
  );
}

const DEVICE_ICONS: Record<BrowserDevice, LucideIcon> = { fit: ScanIcon, mobile: SmartphoneIcon, tablet: TabletIcon, desktop: MonitorIcon };
const DEVICE_NAMES = { fit: "documents.deviceFit", mobile: "documents.deviceMobile", tablet: "documents.deviceTablet", desktop: "documents.deviceDesktop" } as const;

/**
 * The device size the page lays out at (PRD browser-width-zoom B10-B14):
 * only an icon while the page fits its area, and the width, with the scale
 * the page is drawn at when the area is narrower, once a size is chosen.
 */
function DeviceButton({ device, drawn, onChoose }: { device: BrowserDevice; drawn: number; onChoose: (device: BrowserDevice) => void }) {
  const { t } = useInterfaceTranslation();
  const label = deviceLabel(device, drawn);
  const size = devicePlacement(device);
  const Icon = device === "fit" ? MonitorSmartphoneIcon : DEVICE_ICONS[device];
  const name = size ? t("documents.deviceSizeAt", { name: t(DEVICE_NAMES[device]), width: size.width }) : t("documents.deviceSize");
  return (
    <DropdownMenu>
      <Hint label={name}>
        <DropdownMenuTrigger asChild>
          <Button variant={size ? "secondary" : "ghost"} size={label ? "sm" : "icon-sm"} className={cn("shrink-0 gap-xs font-mono text-caption", size ? "px-xs" : "text-muted-foreground")} aria-label={name} data-browser-device-button={device}>
            <Icon />
            {label ? <span className="whitespace-nowrap" data-browser-device-label="true">{label}</span> : null}
          </Button>
        </DropdownMenuTrigger>
      </Hint>
      <DropdownMenuContent align="end" data-browser-device-menu="true">
        {BROWSER_DEVICES.map((choice) => {
          const ChoiceIcon = DEVICE_ICONS[choice];
          const width = devicePlacement(choice)?.width;
          return (
            <Fragment key={choice}>
              <DropdownMenuCheckboxItem checked={choice === device} onSelect={() => onChoose(choice)} data-browser-device-choice={choice}>
                <ChoiceIcon />
                <span className="whitespace-nowrap">{t(DEVICE_NAMES[choice])}</span>
                <span className="ml-auto pl-md font-mono text-caption text-muted-foreground">{width ?? t("documents.deviceFitWidth")}</span>
              </DropdownMenuCheckboxItem>
              {choice === "fit" ? <DropdownMenuSeparator /> : null}
            </Fragment>
          );
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/**
 * The page's address, shown without a web scheme until it is edited so a
 * narrow area still shows the host; focusing it shows the whole address
 * selected. Return loads what was typed, Escape puts the page's address back.
 */
function AddressField({ address, onSubmit, autoFocus = false, trailing = null }: { address: string; onSubmit: (url: string) => void; autoFocus?: boolean; trailing?: ReactNode }) {
  const { t } = useInterfaceTranslation();
  const [draft, setDraft] = useState<string | null>(null);
  const field = useRef<HTMLInputElement>(null);
  // Focus swaps the shown address for the whole one, which drops any
  // selection; selecting in the commit that swaps it means the first key
  // after focus replaces the address rather than landing after it.
  const selectOnCommit = useRef(false);
  useLayoutEffect(() => {
    if (!selectOnCommit.current) return;
    selectOnCommit.current = false;
    field.current?.select();
  });
  return (
    <div className="relative flex min-w-0 flex-1 items-center">
      <Input
        ref={field}
        mono
        className={cn("h-(--size-control-sm) flex-1 text-caption", trailing && "pr-xxxl")}
        aria-label={t("documents.pageAddress")}
        placeholder={t("documents.enterUrl")}
        autoFocus={autoFocus}
        spellCheck={false}
        autoComplete="off"
        value={draft ?? addressShown(address)}
        data-browser-address="true"
        onFocus={() => {
          selectOnCommit.current = true;
          setDraft(address);
        }}
        onChange={(event) => setDraft(event.currentTarget.value)}
        onBlur={() => setDraft(null)}
        onKeyDown={(event) => {
          if (event.nativeEvent.isComposing) return;
          if (event.key === "Enter") {
            event.preventDefault();
            const url = addressUrl(draft ?? address);
            setDraft(null);
            if (url) onSubmit(url);
            event.currentTarget.blur();
          } else if (event.key === "Escape" && draft !== null && draft !== address) {
            event.preventDefault();
            event.stopPropagation();
            setDraft(address);
          }
        }}
      />
      {trailing ? <div className="absolute inset-y-0 right-xs flex items-center">{trailing}</div> : null}
    </div>
  );
}

/**
 * The page-side half of the host's browser views, mounted once: it tells the
 * host which browser displays the front Workspace holds, records what each
 * page says in the core (its address and title), turns a page's new window
 * into another browser display, and makes a clicked page's area the active one.
 */
export function BrowserHost({ actions }: { actions: Actions }) {
  const connection = useShellStore((s) => s.connection);
  const view = useShellStore((s) => workspaceViewOf(s.rest));
  const inventory = useShellStore((s) => s.rest?.browser_views);
  const scopes = useShellStore((s) => s.rest?.browser_scopes);
  const front = view ? workspaceKey({ device_id: view.device_id, path: view.path }) : null;
  const layout = view?.layout ?? null;
  useEffect(() => {
    syncBrowserFront(front ? parseWorkspaceKey(front) : null, layout, inventory ?? [], scopes ?? []);
    // A display the core no longer keeps takes its device size with it.
    if (inventory) useUiStore.getState().keepBrowserDevices(new Set(inventory.map((row) => hostKey(workspaceKey(row), row.view_id))));
  }, [front, layout, inventory, scopes]);
  useEffect(() => {
    if (connection !== "live") clearBrowserAttachments();
  }, [connection]);

  const latest = useRef(actions);
  latest.current = actions;
  useEffect(() => {
    const bridge = browserBridge();
    if (!bridge) return undefined;
    const unsubscribe = bridge.onEvent((event) => {
      if (event.kind === "state") {
        notePageState(event);
        const workspace = parseWorkspaceKey(event.workspace);
        if (workspace) latest.current.reportBrowserState(workspace, event.id, event.state.url, event.state.title, event.load, event.state.loading, event.state.failure, true);
      } else if (event.kind === "gone") {
        noteBrowserAttachment(event);
        const workspace = parseWorkspaceKey(event.workspace);
        if (workspace) latest.current.reportBrowserState(workspace, event.id, event.url, "", event.load, false, null, false);
      } else if (event.kind === "attached") {
        noteBrowserAttachment(event);
      } else if (event.kind === "open") {
        const workspace = parseWorkspaceKey(event.workspace);
        if (workspace) latest.current.openBrowser(event.url, workspace, undefined, event.id);
      } else if (event.kind === "focus") {
        const current = workspaceViewOf(useShellStore.getState().rest);
        if (!current?.layout || workspaceKey({ device_id: current.device_id, path: current.path }) !== event.workspace) return;
        const located = locateDisplay(current.layout.root, event.id);
        if (located && (current.layout.active_area !== located.area.id || located.area.active !== event.id)) latest.current.focusView(event.id);
      }
    });
    const unmountAttachments = mountBrowserAttachments();
    return () => {
      unsubscribe();
      unmountAttachments();
    };
    // `record` reads everything through the stores and refs.
  }, []);
  return null;
}

/** The area that holds a display in the front Workspace's layout, or none. */
function useAreaOf(displayId: string): string | null {
  return useShellStore((s) => {
    const layout = workspaceViewOf(s.rest)?.layout;
    return layout ? (locateDisplay(layout.root, displayId)?.area.id ?? null) : null;
  });
}

/**
 * Why Comment can end without the operator's own turn-off, in words they can
 * act on, keyed by what hided answers; a failure the operator cannot act on
 * falls to `feedback.unavailable.other` with its reason.
 */
const COMMENT_STOPS = {
  display_busy: "display_busy",
  comment_limit: "comment_limit",
  comment_unavailable_here: "comment_unavailable_here",
  browser_control_unavailable: "browser_control_unavailable",
  browser_control_busy: "browser_control_busy",
  display_missing: "display_missing",
  display_closed: "display_closed",
  page_unresponsive: "page_unresponsive",
  dialog_open: "dialog_open",
} as const;

function isCommentStop(reason: string): reason is keyof typeof COMMENT_STOPS {
  return Object.hasOwn(COMMENT_STOPS, reason);
}

/**
 * Comment on a page (PRD view-opener-feedback B9-B11, B32): while it is on,
 * hided holds the page and draws the overlay in it, so a click or a drag
 * there picks a place rather than reaching the page, and the note on it is
 * written in the window (`PageNoteLayer`). The pins are the page's own
 * comments, those left at the address it shows now; they are sent again
 * whenever the core's comments change. Turning it on gives the page the
 * keyboard, so Esc there ends it as the hint says; so do Comment again and
 * leaving the View.
 */
function BrowserCommentButton({ display, workspace, areaId, address, actions }: { display: ViewDisplaySnapshot; workspace: string; areaId: string | null; address: string; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const view = parseWorkspaceKey(workspace);
  const key = view ? browserCommentKey(view, display.id) : null;
  const mode = useFeedbackStore((s) => (key ? s.browser[key] : undefined) ?? null);
  // Comment shows on only once hided holds the page and its overlay is in:
  // before that, a press in the page would still reach the page.
  const starting = mode?.on === true && mode.starting === true;
  const on = mode?.on === true && !starting;
  const feedback = useViewFeedback();
  const comments = commentsOn(feedback, display.id);
  const pins = comments
    .filter((comment) => comment.url === address && (comment.anchor.kind === "element" || comment.anchor.kind === "area"))
    .map((comment) => ({
      id: comment.id,
      number: comment.number,
      rect: comment.anchor.kind === "element" || comment.anchor.kind === "area" ? comment.anchor.rect : null,
      note: comment.note,
      late: comment.not_delivered,
      area: comment.anchor.kind === "area",
    }));
  const strings = { hint: t("feedback.hint.browser") };
  const request = JSON.stringify({ pins, strings });
  // `update` only redraws the pins of a session still on: hided never starts
  // one for it, so pins that cross the page's own Esc cannot turn Comment on again.
  const send = (turnOn: boolean, update = false) => {
    if (!view || !areaId) return;
    actions.browserComment({ workspace: view, area_id: areaId, display_id: display.id, on: turnOn, ...(update ? { update: true } : {}), ...(turnOn ? JSON.parse(request) : {}) });
  };
  const sendRef = useRef(send);
  sendRef.current = send;
  // The page's pins follow the core's comments while Comment is on.
  const drawn = useRef(request);
  useEffect(() => {
    if (on && drawn.current !== request) sendRef.current(true, true);
    drawn.current = request;
  }, [on, request]);
  // Leaving the View ends its Comment, one still starting too.
  useEffect(
    () => () => {
      if (key && useFeedbackStore.getState().browser[key]?.on) {
        sendRef.current(false);
        noteBrowserComment(key, { on: false, reason: null });
      }
    },
    [key],
  );
  const reason = !on && mode?.reason ? mode.reason : null;
  const words = reason
    ? isCommentStop(reason)
      ? t(`feedback.unavailable.${COMMENT_STOPS[reason]}`)
      : t("feedback.unavailable.other", { reason })
    : on || starting
      ? t("feedback.commentStop")
      : t("feedback.commentOn");
  return (
    <Hint label={words} shortcut={on ? "Esc" : undefined}>
      <Button
        variant={on ? "default" : "ghost"}
        size="sm"
        className={on ? "" : reason ? "text-warning" : "text-muted-foreground"}
        aria-pressed={on}
        aria-busy={starting}
        disabled={!areaId || !view}
        data-browser-comment={on ? "on" : starting ? "starting" : "off"}
        onClick={() => {
          // A press while it starts takes the turn-on back.
          const turnOn = !on && !starting;
          if (key) noteBrowserComment(key, turnOn ? { on: true, reason: null, starting: true } : { on: false, reason: null });
          send(turnOn);
          if (turnOn) focusBrowserDisplay(workspace, display.id);
        }}
      >
        <MessageSquarePlusIcon />
        {t("feedback.comment")}
      </Button>
    </Hint>
  );
}
