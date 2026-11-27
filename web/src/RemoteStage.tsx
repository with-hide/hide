// A paired machine's own Hide screen in this window (PRD device-connect-iroh
// B17, B20, B21, B63): the rail stays, and the rest of the window is that
// computer's screen, which the desktop host draws in a sandboxed view of its
// own over the place this stage holds. While it cannot show (not connected,
// paused, update needed, not allowed) the stage says why in that state's own
// words, and the screen opens on its own once it can. A layer of this
// window (a menu, Settings) is drawn over the stage only while the screen
// stands aside, since a native view covers every page under it.

import { LoaderCircleIcon } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { Actions } from "./actions";
import { overlayRects, intersects } from "./browserViews";
import { DeviceRail } from "./components/device-rail";
import { Button } from "./components/ui/button";
import { screenState, screenStateKey, type ScreenState } from "./computers";
import { ownDisplayPreferences, sameDisplayPreferences, type DisplayPreferences } from "./displayPreferences";
import { railShown } from "./devices";
import { hostBridge, type BrowserRect, type RemoteScreenState } from "./host";
import { useInterfaceTranslation } from "./i18n/client";
import { pairedMachines } from "./pairing";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

/** How long a page that could not load waits before it is asked again while its machine reads as reachable. */
const RETRY_MS = 5_000;

/**
 * How long a screen may take to open before a relay connection is named as
 * the likely reason: a direct or Tailscale screen shows within a second or
 * two, a relayed one can take a few seconds more, so past this the operator
 * is told it is the relay rather than left to think it is stuck.
 */
export const SLOW_OPENING_MS = 5_000;

/** The state a failed load stands for, from the reason this computer's daemon gave (`hided/src/remote_screen.rs`). */
export function failedState(reason: string | undefined): Exclude<ScreenState, "ready"> {
  if (reason === "paused") return "paused";
  if (reason === "paused_there") return "paused_there";
  if (reason === "no_grant" || reason === "not_paired" || reason === "refused") return "no_grant";
  return "not_connected";
}

export function RemoteStage({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const device = useUiStore((s) => s.remoteScreen);
  const peer = useShellStore((s) => pairedMachines(s.rest).find((row) => row.device_id === device) ?? null);
  // The key the machine is paired under now: a page is the screen of that key only.
  const key = peer?.key ?? null;
  // What this window calls the machine, which its page names itself by.
  const name = peer?.name ?? null;
  // This window's View menu choices, which that computer's screen draws with while this window shows it.
  const logoMode = useShellStore((s) => ownDisplayPreferences(s.rest?.ui_state).agent_logo_mode);
  const turnRowMode = useShellStore((s) => ownDisplayPreferences(s.rest?.ui_state).pane_turn_row_mode);
  const display = useRef<DisplayPreferences>({ agent_logo_mode: logoMode, pane_turn_row_mode: turnRowMode });
  display.current = { agent_logo_mode: logoMode, pane_turn_row_mode: turnRowMode };
  // The last show sent, so a changed choice reaches the open screen without closing it.
  const shown = useRef<{ device: string; key: string; rect: BrowserRect; name: string | null; display: DisplayPreferences } | null>(null);
  const rail = useShellStore((s) => railShown(s.rest));
  const bridge = hostBridge()?.remoteScreens ?? null;
  const stage = useRef<HTMLDivElement>(null);
  const [loaded, setLoaded] = useState<RemoteScreenState | null>(null);
  const [attempt, setAttempt] = useState(0);
  const state = peer ? screenState(peer) : "no_grant";

  // A machine no longer paired leaves no screen to come back to: the window
  // shows its own again, which the daemon is told (`windowScreen.ts`).
  useEffect(() => {
    if (device !== null && peer === null) {
      useUiStore.getState().setRemoteScreen(null);
      actions.noteWindowScreen();
    }
  }, [device, peer, actions]);

  useEffect(() => {
    setLoaded(null);
    if (!bridge || device === null) return undefined;
    return bridge.onState((reported) => {
      if (reported.device === device) setLoaded(reported);
    });
  }, [bridge, device]);

  // A page that could not load while its machine reads as reachable is
  // asked again, and again when the machine's state moves (B21).
  const failed = loaded?.state === "failed";
  useEffect(() => {
    if (!failed || state !== "ready") return undefined;
    const timer = window.setTimeout(() => setAttempt((count) => count + 1), RETRY_MS);
    return () => window.clearTimeout(timer);
  }, [failed, state, attempt]);

  // The screen stands over the stage, from the rail's edge to the window's,
  // top to bottom, while its machine can show it and nothing of this window
  // is drawn over the stage; otherwise it stands aside and its socket closes.
  const showing = bridge !== null && device !== null && key !== null && state === "ready";
  useEffect(() => {
    if (!bridge) return undefined;
    if (!showing || device === null || key === null) {
      shown.current = null;
      bridge.show(null);
      return undefined;
    }
    let frame = 0;
    let last = "";
    const place = () => {
      frame = 0;
      const element = stage.current;
      if (!element) return;
      const left = element.getBoundingClientRect().left;
      const rect: BrowserRect = { x: left, y: 0, width: Math.max(0, window.innerWidth - left), height: window.innerHeight };
      const covered = overlayRects().rects.some((overlay) => intersects(overlay, rect));
      const placed = covered ? "covered" : JSON.stringify(rect);
      if (placed === last) return;
      last = placed;
      shown.current = covered ? null : { device, key, rect, name, display: display.current };
      bridge.show(shown.current);
    };
    const schedule = () => {
      if (frame === 0) frame = requestAnimationFrame(place);
    };
    place();
    const resized = new ResizeObserver(schedule);
    if (stage.current) resized.observe(stage.current);
    const layers = new MutationObserver(schedule);
    layers.observe(document.body, { childList: true });
    window.addEventListener("resize", schedule);
    return () => {
      if (frame !== 0) cancelAnimationFrame(frame);
      resized.disconnect();
      layers.disconnect();
      window.removeEventListener("resize", schedule);
      shown.current = null;
      bridge.show(null);
    };
  }, [bridge, showing, device, key, name, attempt]);

  // A choice changed in this window's View menu reaches the screen it shows at once.
  useEffect(() => {
    const next = { agent_logo_mode: logoMode, pane_turn_row_mode: turnRowMode };
    if (!bridge || !shown.current || sameDisplayPreferences(shown.current.display, next)) return;
    shown.current = { ...shown.current, display: next };
    bridge.show(shown.current);
  }, [bridge, logoMode, turnRowMode]);

  const why = state !== "ready" ? state : failed ? failedState(loaded?.reason) : null;
  // Opening: the machine can show and its screen has not shown yet.
  const opening = why === null && loaded?.state !== "shown";
  const [slow, setSlow] = useState(false);
  // Another computer chosen while one opens stays opening, so its device starts the count over.
  useEffect(() => {
    setSlow(false);
    if (!opening) return undefined;
    const timer = window.setTimeout(() => setSlow(true), SLOW_OPENING_MS);
    return () => window.clearTimeout(timer);
  }, [opening, device]);
  return (
    <>
      {rail ? <DeviceRail actions={actions} /> : null}
      <div ref={stage} className="flex min-h-0 min-w-0 flex-1 flex-col items-center justify-center gap-sm p-xl text-center" data-remote-stage={device ?? ""} data-remote-state={why ?? loaded?.state ?? "opening"}>
        {why ? (
          <>
            <span className="text-headline font-semibold text-foreground">{peer?.name ?? ""}</span>
            <span className="text-body text-muted-foreground">{t(screenStateKey(why))}</span>
            {why === "not_connected" ? <span className="text-caption text-muted-foreground">{t("remoteScreen.reconnects")}</span> : null}
            {/* Which side must update is acted on in the window's notice (PeerUpdateNotice), so the stage only says it. */}
            {why !== "not_connected" && why !== "confirming" && why !== "update_here" && why !== "update_there" ? (
              <Button variant="secondary" size="sm" onClick={() => actions.openSettings("devices")} data-remote-settings="true">
                {t("remoteScreen.openSettings")}
              </Button>
            ) : null}
          </>
        ) : opening ? (
          <>
            <LoaderCircleIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-muted-foreground motion-safe:animate-spin" />
            <span className="text-headline font-semibold text-foreground">{peer?.name ?? ""}</span>
            {/* A status region from the start, so the relay line is read out when it appears. */}
            <div role="status" className="flex flex-col items-center gap-sm">
              <span className="text-caption text-muted-foreground">{t("remoteScreen.opening")}</span>
              {slow && peer?.path === "relay" ? <span className="text-caption text-muted-foreground" data-remote-slow="relay">{t("remoteScreen.slowRelay")}</span> : null}
            </div>
          </>
        ) : null}
      </div>
    </>
  );
}
