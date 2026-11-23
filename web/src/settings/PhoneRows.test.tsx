// @vitest-environment jsdom
// Settings › Devices' phone group (operator review 2026-11-23; UI_BEHAVIOR.md,
// Settings › Devices' phones): a phone is one more device, Add device › 폰 is
// the one way to pair one, and a screen another machine shows of this one
// lists the phones with 연결 해제 alone.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import type { MobileState, PhoneRow } from "../mobileSettings";
import { NOTHING_UNDER_WAY } from "../pairing";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import type { DispatchFn } from "../ws";
import { DevicesTab } from "./DevicesTab";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

const passed = { installed: "ok", logged_in: "ok", https: "ok", host_name: "mac" } as const;
const phone = (over: Partial<PhoneRow> = {}): PhoneRow => ({ id: "p1", name: "iPhone", last_seen_ms: 0, connected: true, notifications: "on", revoke_at_ms: 7 * 86_400_000, ...over });

function frame(over: Partial<MobileState> = {}): MobileState {
  return {
    method: "relay",
    relay: { state: "connected" },
    enabled: true,
    exposure: "exposed",
    checklist: null,
    download_url: "https://tailscale.com/download",
    admin_url: "https://login.tailscale.com/admin/dns",
    foreign_target: null,
    failure: null,
    url: "https://app.withhide.dev/",
    qr: null,
    code_expires_at_ms: null,
    phones: [phone()],
    max_phones: 4,
    push_mode: "app_closed",
    now_ms: 0,
    ...over,
  };
}

afterEach(() => {
  document.body.innerHTML = "";
  vi.useRealTimers();
});

const LOCAL = { agent_scope: { kind: "none" }, id: "local", label: null, kind: "local", state: "local", message: null, ssh_alias: null, agent_count: 0, test: null };

/** `own`: this machine's own window, which hided sends the `pairing` frame; a screen another machine shows of this one gets none. */
async function mount(mobile: MobileState | null, own = true) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  vi.useFakeTimers({ toFake: ["Date"] });
  // jsdom lays nothing out, so it has no scrolling.
  Element.prototype.scrollIntoView = () => undefined;
  vi.setSystemTime(0);
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const saved = useShellStore.getState();
  await act(async () => {
    useShellStore.setState({ connection: "live", mobile, pairing: own ? NOTHING_UNDER_WAY : null, rest: { navigator: { focused_device_id: "local", devices: [LOCAL], peers: [] }, status: { remote: [] }, ui_state: {} } } as never);
    root.render(<TooltipProvider><DevicesTab actions={actions} /></TooltipProvider>);
  });
  const q = (selector: string) => document.body.querySelector(selector) as HTMLElement | null;
  const press = async (selector: string) => { await act(async () => { q(selector)?.click(); }); };
  const set = async (next: Partial<MobileState>) => { await act(async () => { useShellStore.setState({ mobile: { ...useShellStore.getState().mobile!, ...next } }); }); };
  return { events, q, press, set, unmount: async () => { await act(async () => root.unmount()); useShellStore.setState(saved, true); } };
}

const escape = () => act(async () => { document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
const kinds = (events: Parameters<DispatchFn>[0][]) => events.map((event) => event.kind).filter((kind) => kind.startsWith("mobile_"));

async function openMenu(q: (selector: string) => HTMLElement | null, id: string) {
  await act(async () => {
    q(`[data-mobile-phone-menu="${id}"]`)?.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  });
}
const menuItems = (id: string) => [...document.querySelectorAll(`[data-mobile-phone-menu-content="${id}"] [role="menuitem"]`)].map((item) => item.textContent);

it("shows a phone as a device row in its own group, with the switch at the group's head and no QR (mockup 3, 4)", async () => {
  const { events, q, unmount } = await mount(frame());
  const group = q('[data-device-section="phones"]')!;
  expect(group.querySelector("h3")?.textContent).toBe("Phones");
  expect(q('[data-mobile-switch="true"]')?.getAttribute("aria-checked")).toBe("true");
  expect(q('[data-mobile-phone-state="p1:connected"]')?.textContent).toBe("●connected");
  expect(q('[data-mobile-phone-line="p1"]')?.textContent).toBe("Receiving notifications");
  // The relay's state and Tailscale are one row; nothing pairs from the group itself.
  expect(q('[data-mobile-method-open="relay"]')?.textContent).toBe("Phone connectionHide relay (Tokyo)");
  expect(group.querySelector("[data-mobile-qr], [data-mobile-show-code], [data-mobile-problem]")).toBeNull();
  // Showing the tab makes no code (B58).
  expect(kinds(events)).toEqual(["mobile_observe"]);
  await openMenu(q, "p1");
  expect(menuItems("p1")).toEqual(["Rename", "Connection details…", "Disconnect…"]);
  await escape();
  await unmount();
});

it("says why phones cannot connect in one line over the rows only while the relay is down or refused (mockup 5)", async () => {
  const { events, q, press, set, unmount } = await mount(frame({ relay: { state: "connecting" } }));
  expect(q("[data-mobile-problem]")).toBeNull();
  await set({ relay: { state: "retrying" } });
  expect(q('[data-mobile-problem="relay"]')?.textContent).toBe("Relay unreachable · RetryingRetry now");
  await press('[data-mobile-retry="true"]');
  expect(kinds(events)).toContain("mobile_retry");
  await set({ relay: { state: "refused", reason: "unsupported_version" } });
  expect(q('[data-mobile-problem="relay"]')?.textContent).toBe("This Hide is too oldUpdate");
  // Switched off, a relay that is off is not a problem to show.
  await set({ enabled: false, relay: { state: "off" }, exposure: "off" });
  expect(q("[data-mobile-problem]")).toBeNull();
  await unmount();
});

it("names a failing Tailscale step over the rows, and keeps the steps and the switch in Phone connection", async () => {
  const { q, press, unmount } = await mount(frame({ method: "tailscale", relay: { state: "off" }, exposure: "checking", checklist: { ...passed, logged_in: "failed" } }));
  expect(q('[data-mobile-problem="tailscale"]')?.textContent).toBe("Not signed in to Tailscale");
  await press('[data-mobile-method-open="tailscale"]');
  expect(q('[data-mobile-method-dialog="tailscale"]')).not.toBeNull();
  expect(q('[data-mobile-step="logged_in"]')).not.toBeNull();
  await unmount();
});

it("asks computer or phone first; Phone turns phone access on, shows one code once ready, and closing ends it (mockup 6, B58)", async () => {
  const { events, q, press, set, unmount } = await mount(frame({ enabled: false, relay: { state: "off" }, exposure: "off", phones: [] }));
  await press('[data-device-add-open="true"]');
  expect([...document.querySelectorAll("[data-add-choice]")].map((choice) => choice.getAttribute("data-add-choice"))).toEqual(["computer", "phone"]);
  expect(q("[data-add-kind]")).toBeNull();
  await press('[data-add-choice="phone"]');
  expect(q("[data-add-device-dialog]")).toBeNull();
  expect(q('[data-mobile-pair-dialog="relay"]')).not.toBeNull();
  expect(kinds(events)).toEqual(["mobile_observe", "mobile_enable"]);
  // No code until the relay is ready, and then one.
  await set({ enabled: true, relay: { state: "connecting" }, exposure: "checking" });
  expect(q('[data-mobile-connection="connecting"]')).not.toBeNull();
  await set({ relay: { state: "connected" }, exposure: "exposed" });
  await set({ qr: "https://app.withhide.dev/#pair=x", code_expires_at_ms: 300_000 });
  expect(kinds(events).filter((kind) => kind === "mobile_show_code")).toHaveLength(1);
  expect(q("[data-mobile-qr]")?.getAttribute("data-mobile-qr")).toBe("https://app.withhide.dev/#pair=x");
  expect(q("[data-mobile-countdown]")?.textContent).toBe("Code expires in 5:00");
  // The phone that scans it closes the window.
  await set({ qr: null, code_expires_at_ms: null, phones: [phone()] });
  expect(q("[data-mobile-pair-dialog]")).toBeNull();
  expect(kinds(events).at(-1)).toBe("mobile_hide_code");
  await unmount();
});

it("offers a fresh code once the shown one expired, a press away", async () => {
  const expired = await mount(frame());
  await expired.press('[data-device-add-open="true"]');
  await expired.press('[data-add-choice="phone"]');
  await expired.set({ qr: "https://app.withhide.dev/#pair=x", code_expires_at_ms: -1 });
  expect(expired.q("[data-mobile-qr]")).toBeNull();
  expect(kinds(expired.events).filter((kind) => kind === "mobile_show_code")).toHaveLength(1);
  await expired.press('[data-mobile-show-code="true"]');
  expect(kinds(expired.events).filter((kind) => kind === "mobile_show_code")).toHaveLength(2);
  await expired.unmount();

});

it("opens Add device on a computer's two ways when the way in adds a computer, and the phone group when it opens a phone's settings", async () => {
  const { q, unmount } = await mount(frame());
  await act(async () => { useUiStore.getState().openAddDevice("computer"); });
  expect(q("[data-add-choose]")).toBeNull();
  expect(q("[data-add-kind]")).not.toBeNull();
  await act(async () => { useUiStore.getState().openPhones(); });
  expect(useUiStore.getState().devicesRequest).toBeNull();
  await unmount();
});

it("renames a phone, keeping the name it paired with, and an empty field gives that one back (mockup 4)", async () => {
  const { events, q, press, unmount } = await mount(frame({ phones: [phone({ name: "Work phone", own_name: "iPhone" })] }));
  await openMenu(q, "p1");
  await press('[data-mobile-rename="p1"]');
  const input = q('[data-device-rename-input="p1"]') as HTMLInputElement;
  expect(input.value).toBe("Work phone");
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, " ");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await press('[data-device-rename-go="p1"]');
  expect(events.find((event) => event.kind === "mobile_rename")?.payload).toEqual({ phone_id: "p1", name: "" });
  await openMenu(q, "p1");
  await press('[data-mobile-details="p1"]');
  expect(q('[data-mobile-own-name="p1"]')?.textContent).toBe("iPhone");
  await unmount();
});

it("disconnects a phone only after its confirm", async () => {
  const { events, q, press, unmount } = await mount(frame());
  await openMenu(q, "p1");
  await press('[data-mobile-revoke-menu="p1"]');
  expect(kinds(events)).not.toContain("mobile_revoke");
  await press('[data-mobile-revoke="p1"]');
  expect(events.find((event) => event.kind === "mobile_revoke")?.payload).toEqual({ phone_id: "p1" });
  await unmount();
});

it("lists the phones with Disconnect alone on a screen another machine shows of this one, and pairs nothing there (D-53, D-70)", async () => {
  const { events, q, unmount } = await mount(frame({ relay: { state: "retrying" } }), false);
  expect(q('[data-mobile-remote="true"]')).not.toBeNull();
  expect(q('[data-mobile-remote-line="true"]')?.textContent).toBe("Turning this on or off and pairing a phone happen on This computer. Here you can revoke a phone.");
  expect(q("[data-mobile-switch], [data-mobile-method-open], [data-device-add-open]")).toBeNull();
  // The relay's reason is still said, and its retry still reaches hided.
  expect(q('[data-mobile-problem="relay"]')).not.toBeNull();
  await openMenu(q, "p1");
  expect(menuItems("p1")).toEqual(["Disconnect…"]);
  await escape();
  // That screen does not watch the page: its leaving would end this machine's code.
  expect(kinds(events)).toEqual([]);
  await unmount();
});
