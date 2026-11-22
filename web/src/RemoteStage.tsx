// A paired machine's own Hide screen in this window (PRD device-connect-iroh
// B17, B20, B21, B63): the rail stays, and the rest of the window is that
// computer's screen, which the desktop host draws in a sandboxed view of its
// own over the place this stage holds. While it cannot show (not connected,
// paused, update needed, not allowed) the stage says why in that state's own
// words, and the screen opens on its own once it can. A layer of this
// window (a menu, Settings) is drawn over the stage only while the screen
// stands aside, since a native view covers every page under it.

import { useEffect, useRef, useState } from "react";
import type { Actions } from "./actions";
import { overlayRects, intersects } from "./browserViews";
import { DeviceRail } from "./components/device-rail";
import { Button } from "./components/ui/button";
import { screenState, screenStateKey, type ScreenState } from "./computers";
import { railShown } from "./devices";
import { hostBridge, type BrowserRect, type RemoteScreenState } from "./host";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import { pairedMachines } from "./pairing";
import { DeviceSwitchStrip } from "./settings/DeviceSwitch";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { offersUpdate, updateNow, useUpdateStatus } from "./updates";

/** How long a page that could not load waits before it is asked again while its machine reads as reachable. */
const RETRY_MS = 5_000;

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
  const rail = useShellStore((s) => railShown(s.rest));
  const bridge = hostBridge()?.remoteScreens ?? null;
  const stage = useRef<HTMLDivElement>(null);
  // The line a switch that just landed says (devices.switch.fresh) stands over the screen's top, which starts under it.
  const notice = useRef<HTMLDivElement>(null);
  const [loaded, setLoaded] = useState<RemoteScreenState | null>(null);
  const [attempt, setAttempt] = useState(0);
  const state = peer ? screenState(peer) : "no_grant";
  // This computer must update: its own app offers it here as its notice does (B22).
  const update = useUpdateStatus();

  // A machine no longer paired leaves no screen to come back to.
  useEffect(() => {
    if (device !== null && peer === null) useUiStore.getState().setRemoteScreen(null);
  }, [device, peer]);

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
      const top = notice.current?.getBoundingClientRect().height ?? 0;
      const rect: BrowserRect = { x: left, y: top, width: Math.max(0, window.innerWidth - left), height: Math.max(0, window.innerHeight - top) };
      const covered = overlayRects().rects.some((overlay) => intersects(overlay, rect));
      const placed = covered ? "covered" : JSON.stringify(rect);
      if (placed === last) return;
      last = placed;
      bridge.show(covered ? null : { device, key, rect, name });
    };
    const schedule = () => {
      if (frame === 0) frame = requestAnimationFrame(place);
    };
    place();
    const resized = new ResizeObserver(schedule);
    if (stage.current) resized.observe(stage.current);
    if (notice.current) resized.observe(notice.current);
    const layers = new MutationObserver(schedule);
    layers.observe(document.body, { childList: true });
    window.addEventListener("resize", schedule);
    return () => {
      if (frame !== 0) cancelAnimationFrame(frame);
      resized.disconnect();
      layers.disconnect();
      window.removeEventListener("resize", schedule);
      bridge.show(null);
    };
  }, [bridge, showing, device, key, name, attempt]);

  const why = state !== "ready" ? state : failed ? failedState(loaded?.reason) : null;
  return (
    <>
      {rail ? <DeviceRail actions={actions} /> : null}
      <div ref={notice} className={cn("fixed top-0 right-0 z-10", rail ? "left-(--size-rail)" : "left-0")} data-remote-notice="true">
        <DeviceSwitchStrip actions={actions} />
      </div>
      <div ref={stage} className="flex min-h-0 min-w-0 flex-1 flex-col items-center justify-center gap-sm p-xl text-center" data-remote-stage={device ?? ""} data-remote-state={why ?? loaded?.state ?? "opening"}>
        {why ? (
          <>
            <span className="text-headline font-semibold text-foreground">{peer?.name ?? ""}</span>
            <span className="text-body text-muted-foreground">{t(screenStateKey(why))}</span>
            {why === "not_connected" ? <span className="text-caption text-muted-foreground">{t("remoteScreen.reconnects")}</span> : null}
            {why === "update_here" && offersUpdate(update) ? (
              <Button variant="secondary" size="sm" onClick={() => updateNow(update)} data-remote-update="true">
                {t("pairing.action.updateHere")}
              </Button>
            ) : why !== "not_connected" && why !== "confirming" ? (
              <Button variant="secondary" size="sm" onClick={() => actions.openSettings("devices")} data-remote-settings="true">
                {t("remoteScreen.openSettings")}
              </Button>
            ) : null}
          </>
        ) : loaded?.state !== "shown" ? (
          <span className="text-caption text-muted-foreground">{t("remoteScreen.opening")}</span>
        ) : null}
      </div>
    </>
  );
}
