// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { MobileTab } from "./MobileTab";
import type { MobileState } from "./mobileSettings";
import { useShellStore } from "./store";
import type { DispatchFn } from "./ws";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

const passed = { installed: "ok", logged_in: "ok", https: "ok", host_name: "mac" } as const;

function frame(over: Partial<MobileState>): MobileState {
  return {
    method: "tailscale", relay: { state: "connected" },
    enabled: true,
    exposure: "exposed",
    checklist: passed,
    download_url: "https://tailscale.com/download",
    admin_url: "https://login.tailscale.com/admin/dns",
    foreign_target: null,
    failure: null,
    url: "https://mac.ts.net",
    qr: null,
    code_expires_at_ms: null,
    phones: [],
    max_phones: 4,
    push_mode: "app_closed",
    now_ms: 0,
    ...over,
  };
}

afterEach(() => {
  document.body.innerHTML = "";
});

async function mount(mobile: MobileState) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const saved = useShellStore.getState();
  await act(async () => {
    useShellStore.setState({ connection: "live", mobile } as never);
    root.render(<MobileTab actions={actions} />);
  });
  const q = (selector: string) => container.querySelector(selector) as HTMLElement | null;
  return { events, q, container, unmount: async () => { await act(async () => root.unmount()); useShellStore.setState(saved, true); } };
}

const kinds = (events: Parameters<DispatchFn>[0][]) => events.map((event) => event.kind);

it("opens with no code and sends no code request: only Show QR asks for one (B58)", async () => {
  const { events, q, unmount } = await mount(frame({}));
  expect(q("[data-mobile-qr]")).toBeNull();
  expect(kinds(events)).toEqual(["mobile_observe"]);
  await act(async () => { q('[data-mobile-show-code="true"]')?.click(); });
  expect(kinds(events)).toEqual(["mobile_observe", "mobile_show_code"]);
  await unmount();
});

it("shows the code with its countdown and a Hide QR that takes it away", async () => {
  const { events, q, unmount } = await mount(frame({ qr: "https://mac.ts.net/m/#pair=x", code_expires_at_ms: 300_000 }));
  expect(q("[data-mobile-qr]")?.getAttribute("data-mobile-qr")).toBe("https://mac.ts.net/m/#pair=x");
  expect(q("[data-mobile-show-code]")).toBeNull();
  await act(async () => { q('[data-mobile-hide-code="true"]')?.click(); });
  expect(kinds(events)).toContain("mobile_hide_code");
  await unmount();
});

it("offers Show QR directly once the code has expired, and a fresh code is one click away", async () => {
  const { events, q, unmount } = await mount(frame({ qr: "https://mac.ts.net/m/#pair=x", code_expires_at_ms: 1_000, now_ms: 2_000 }));
  expect(q("[data-mobile-qr]")).toBeNull();
  expect(q("[data-mobile-countdown]")).toBeNull();
  expect(q("[data-mobile-hide-code]")).toBeNull();
  await act(async () => { q('[data-mobile-show-code="true"]')?.click(); });
  expect(kinds(events)).toContain("mobile_show_code");
  await unmount();
});

it("collapses a passed Tailscale check to one ready line (B57)", async () => {
  const { q, container, unmount } = await mount(frame({}));
  expect(q('[data-mobile-ready="true"]')?.textContent).toContain("Tailscale is ready");
  expect(container.querySelectorAll("[data-mobile-step]")).toHaveLength(0);
  await unmount();
});

it("shows only the failing step and its fix, and no Pair a phone row until it is fixed (B57)", async () => {
  const { q, container, unmount } = await mount(frame({ exposure: "blocked", checklist: { installed: "ok", logged_in: "ok", https: "failed", host_name: "mac" } }));
  expect(Array.from(container.querySelectorAll("[data-mobile-step]")).map((step) => step.getAttribute("data-mobile-step"))).toEqual(["https"]);
  expect(q("[data-mobile-ready]")).toBeNull();
  expect(q("[data-mobile-pairing]")).toBeNull();
  await unmount();
});

it("keeps Tailscale under Advanced after relay becomes the default (B6)", async () => {
  const { container, unmount } = await mount(frame({}));
  expect(container.querySelector("details")?.textContent).toContain("Tailscale");
  expect(container.querySelector("details")?.open).toBe(false);
  expect(container.textContent).not.toContain("removes only its own entry");
  await unmount();
});

it("leaves the push mode to Settings > Notifications (PRD settings-left-nav B7)", async () => {
  const { q, unmount } = await mount(frame({ push_mode: "off" }));
  expect(q("[data-push-select]")).toBeNull();
  await unmount();
});

it.each([
  ["connecting", undefined, "Connecting…"],
  ["connected", undefined, "Connected · Hide relay (Tokyo)"],
  ["retrying", undefined, "Relay unreachable · Retrying"],
  ["refused", "unsupported_version", "This Hide is too old"],
  ["refused", "daily_limit", "Today’s relay data limit was reached"],
] as const)("shows %s in one connection row without a popup", async (status, reason, text) => {
  const { container, events, unmount } = await mount(frame({ method: "relay", relay: { state: status, reason }, checklist: null }));
  expect(container.textContent).toContain(text);
  expect(container.textContent).toContain("Only your phone and this device can read the contents.");
  expect(container.querySelectorAll("[data-mobile-connection]")).toHaveLength(1);
  expect(container.querySelector('[role="alert"], [role="dialog"]')).toBeNull();
  expect(container.querySelector("[data-mobile-step]")).toBeNull();
  const button = Array.from(container.querySelectorAll("button")).find(b => b.textContent === "Retry now");
  if (status === "retrying") {
    await act(async () => button?.click());
    expect(kinds(events)).toContain("mobile_retry");
  }
  await unmount();
});

it("keeps a direct cleanup failure in the connection line while relay pairing remains available", async () => {
  const { container, q, unmount } = await mount(frame({
    method: "relay", relay: { state: "connected" }, checklist: null,
    failure: { step: "remove", message: "Tailscale CLI unavailable" },
  }));
  expect(container.querySelectorAll("[data-mobile-connection]")).toHaveLength(1);
  expect(container.querySelector("[data-mobile-connection]")?.textContent).not.toContain("Connected · Hide relay");
  expect(q('[data-mobile-show-code="true"]')).not.toBeNull();
  expect(container.querySelector('[role="alert"], [role="dialog"]')).toBeNull();
  await unmount();
});
