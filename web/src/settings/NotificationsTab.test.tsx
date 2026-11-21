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

it("pauses phone notifications by default and lets the phone own receiving", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const container = document.body.appendChild(document.createElement("div"));
  const root = createRoot(container);
  const saved = useShellStore.getState();
  const sent: unknown[] = [];
  const actions = createActions((event) => { sent.push(event); return true; });
  try {
    await act(async () => {
      useShellStore.setState({ connection: "live", mobile: mobile("app_closed") } as never);
      root.render(<NotificationsTab actions={actions} />);
    });
    const toggle = container.querySelector<HTMLElement>('[role="switch"]')!;
    expect(container.querySelector('[data-settings-group="notifications"]')).toBeNull();
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    expect(container.textContent).toContain("Choose whether to receive notifications in the phone app settings.");
    await act(async () => toggle.click());
    expect(sent).toEqual([{ schema_version: 2, kind: "mobile_push_mode", payload: { mode: "always" } }]);
    await act(async () => useShellStore.setState({ mobile: mobile("always") }));
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    await act(async () => toggle.click());
    expect(sent.at(-1)).toEqual({ schema_version: 2, kind: "mobile_push_mode", payload: { mode: "app_closed" } });
  } finally {
    await act(async () => root.unmount());
    useShellStore.setState(saved, true);
  }
});
