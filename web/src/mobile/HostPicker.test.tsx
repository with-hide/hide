// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { HostPicker } from "./HostPicker";
import type { PhoneDevice } from "./protocol";
import { patch } from "./store";

afterEach(() => { patch({ devices: [], connected: false, demo: false }); vi.unstubAllGlobals(); });

// The home head is one line: the computer's name without `.local`, its connection dot, and `+N` for the devices the Mac shows
// through it, while its accessible name keeps the full `name + devices` that a screen reader and a test press it by.
it("draws the home head on one line with the name, the dot and +N for the devices shown through the Mac", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const devices: PhoneDevice[] = [
    { device_id: "local", label: null, local: true, connected: true, needs_you: 1, working: 0 },
    { device_id: "mini", label: "Mac mini", connected: true, needs_you: 0, working: 0, link: "ssh" },
    { device_id: "window", label: "window", connected: false, needs_you: 0, working: 0, link: "ssh" },
  ];
  patch({ devices, connected: true, demo: false });
  const container = document.body.appendChild(document.createElement("div"));
  const root = createRoot(container);
  await act(async () => root.render(<HostPicker name="hoyeonui-MacBookPro.local" />));
  try {
    const head = container.querySelector<HTMLElement>("[data-phone-host-picker]")!;
    expect(head.getAttribute("aria-label")).toBe("hoyeonui-MacBookPro.local + Mac mini");
    expect(head.textContent).toBe("hoyeonui-MacBookPro+1");
    expect(head.querySelector("[data-phone-head-dot]")?.getAttribute("data-phone-head-dot")).toBe("on");
    expect(document.getElementById(head.getAttribute("aria-describedby")!)?.textContent).toBe("Connected");
    await act(async () => patch({ connected: false, devices: devices.slice(0, 1) }));
    expect(head.querySelector("[data-phone-head-dot]")?.getAttribute("data-phone-head-dot")).toBe("off");
    expect(head.querySelector("[data-phone-head-through]")).toBeNull();
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
});
