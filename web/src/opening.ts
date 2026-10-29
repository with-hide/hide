// Bringing a Workspace forward on the page: what was asked for is
// remembered as an open, and `CenterScreen` shows it once the core has it in
// front, or says why it did not come (`navigation.ts`, openingProgress).

import type { OpenTarget } from "./navigation";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

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
