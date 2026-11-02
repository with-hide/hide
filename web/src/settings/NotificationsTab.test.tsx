// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import type { MobileState } from "../mobileSettings";
import { useShellStore } from "../store";
import { NotificationsTab } from "./NotificationsTab";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

const mobile = (push_mode: MobileState["push_mode"]): MobileState => ({
  method: "tailscale", relay: { state: "connected" },
  enabled: false,
  exposure: "off",
  checklist: null,
  download_url: "https://tailscale.com/download",
  admin_url: "https://login.tailscale.com/admin/dns",
  foreign_target: null,
  failure: null,
  url: null,
  qr: null,
  code_expires_at_ms: null,
  phones: [],
  max_phones: 4,
  push_mode,
  now_ms: 0,
});

afterEach(() => {
  document.body.innerHTML = "";
});

it("picks the phone push mode in one dropdown that describes only the chosen mode, in a browser tab too (B6, mobile-companion B59)", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const saved = useShellStore.getState();
  await act(async () => {
    useShellStore.setState({ connection: "live", mobile: mobile("off") } as never);
    root.render(<NotificationsTab actions={createActions(() => true)} />);
  });
  const q = (selector: string) => container.querySelector(selector) as HTMLElement | null;
  // A browser tab has no desktop notifications (first-run-onboarding D-14): the phone group is the page.
  expect(q('[data-settings-group="notifications"]')).toBeNull();
  expect(q('[data-settings-group="phone"] [data-push-select]')).not.toBeNull();
  expect(q("[data-push-detail]")?.getAttribute("data-push-detail")).toBe("off");
  expect(container.textContent).not.toContain("Send to your phone only while desktop hide isn't connected");
  await act(async () => root.unmount());
  useShellStore.setState(saved, true);
});
