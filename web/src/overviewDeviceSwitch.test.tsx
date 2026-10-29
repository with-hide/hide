// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "./components/ui/tooltip";
import { clientI18n } from "./i18n/translator";
import { DeviceSwitch } from "./Overview";
import type { SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";

const WSL = "wsl-ubuntu-1a2b3c4d";

// This PC, its WSL Ubuntu and an SSH device, both connected (PRD wsl-device B5).
const rest = {
  navigator: {
    focused_device_id: WSL,
    devices: [
      { id: "local", label: null, kind: "local", state: "local", message: null },
      { id: WSL, label: "WSL Ubuntu", kind: "remote", state: "ready", message: null, ssh_alias: null, wsl_distro: "Ubuntu" },
      { id: "mini", label: "mini", kind: "remote", state: "ready", message: null, ssh_alias: "mini" },
    ],
    workspaces: [],
  },
  status: { remote: [WSL, "mini"].map((id) => ({ target_id: id, state: "connected", session: null })) },
} as unknown as SnapshotRest;

describe("the Overview's device switch", () => {
  let container: HTMLDivElement;
  let root: Root;
  let saved: ReturnType<typeof useShellStore.getState>;

  beforeEach(async () => {
    saved = useShellStore.getState();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    // A WSL distribution is on a Windows PC, which the window names This PC.
    vi.spyOn(window.navigator, "platform", "get").mockReturnValue("Win32");
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    await act(async () => { await clientI18n.changeLanguage("ko"); });
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(saved, true);
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("names a WSL device with its letters and where it is, on the button and in its menu, and an SSH device without it", async () => {
    await act(async () => {
      useShellStore.setState({ connection: "live", rest });
      root.render(<TooltipProvider><DeviceSwitch selected={WSL} /></TooltipProvider>);
    });
    const trigger = container.querySelector<HTMLButtonElement>("[data-overview-devices] button")!;
    expect(trigger.textContent).toContain("WU");
    expect(trigger.textContent).toContain("WSL Ubuntu");
    expect(trigger.querySelector("[data-device-place]")?.textContent).toBe("이 PC");
    await act(async () => {
      // jsdom has no PointerEvent; Radix reads only the button and the modifier keys.
      trigger.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, button: 0 }));
      trigger.click();
    });
    const choice = (id: string) => document.body.querySelector(`[data-overview-device-choice="${id}"]`);
    expect(choice(WSL)?.querySelector("[data-device-place]")?.textContent).toBe("이 PC");
    expect(choice(WSL)?.textContent).toContain("WU");
    expect(choice("mini")?.textContent).toContain("mini");
    expect(choice("mini")?.querySelector("[data-device-place]")).toBeNull();
  });
});
