// Bringing a Workspace forward on the page: what was asked for is
// remembered as an open, and `CenterScreen` shows it once the core has it in
// front, or says why it did not come (`navigation.ts`, openingProgress).

import type { OpenTarget } from "./navigation";
import { remoteTargetOfPane } from "./remote";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

/**
 * After a start's answer names its pane: a device's pane is opened from the
 * page once it is listed, through that device's own focus. One on this
 * machine is the core's to bring forward: its start holds the follow and
 * cancels it on a newer visit, a changed connection or the 90-second bound,
 * and the action that sent the start began the opening that shows what the
 * core selected, so the page sends no focus of its own.
 */
export function followStartedPane(paneId: string): void {
  if (remoteTargetOfPane(useShellStore.getState().rest, paneId)) useUiStore.getState().setFocusWhenListed(paneId);
}

/** Remembers what was asked to come forward; `CenterScreen` shows it once it is in front. */
export function beginOpening(target: OpenTarget): void {
  const ui = useUiStore.getState();
  useUiStore.setState({
    overviewOpen: false,
    overviewReturnFocus: ui.overviewOpen ? document.querySelector<HTMLElement>('[data-pane-view][data-focused="true"] .xterm-helper-textarea') : null,
    opening: { target, errorBefore: useShellStore.getState().rest?.status?.last_error?.occurred_at ?? null, failure: null },
  });
}

/**
 * A `window_call` frame: `hide <path>` brought a project in front in the
 * core, and the page leaves All projects or an Overview for it. A device's
 * new project is not listed until its helper confirms the folder, so it is
 * matched by its folder rather than an id.
 */
export function showProjectOpened(payload: Record<string, unknown> | undefined): void {
  const { device_id: deviceId, path, checkout_id: checkoutId } = payload ?? {};
  if (typeof deviceId !== "string" || typeof path !== "string") return;
  beginOpening({ checkoutId: typeof checkoutId === "string" ? checkoutId : "", deviceId, path });
}
