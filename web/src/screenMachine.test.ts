// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { awayDevice, coreMachineName, machineName, screenNodeFromHash, systemMachineKey, windowFirst } from "./screenMachine";

describe("the screen's own machine (PRD core-host-node-remote-core B13)", () => {
  it("is the node the hash names beside the token", () => {
    expect(screenNodeFromHash("#token=abc&node=14ad0319-60d7-5a7c-ae5a-f71550dc8a60")).toBe("14ad0319-60d7-5a7c-ae5a-f71550dc8a60");
  });

  it("is the core's own when the hash names none", () => {
    expect(screenNodeFromHash("#token=abc")).toBeNull();
    expect(screenNodeFromHash("")).toBeNull();
  });

  it("is no node when the value cannot be a node id", () => {
    expect(screenNodeFromHash("#node=remote:mini:pane:w1")).toBeNull();
    expect(screenNodeFromHash("#node=")).toBeNull();
    expect(screenNodeFromHash(`#node=${"a".repeat(129)}`)).toBeNull();
  });
});

describe("systemMachineKey", () => {
  it("names a machine by the system its device row reports, not the one the shell runs on", () => {
    vi.stubGlobal("navigator", { platform: "MacIntel" });
    expect(systemMachineKey("macos aarch64")).toBe("common.thisMac");
    expect(systemMachineKey("windows x86_64")).toBe("common.thisPc");
    expect(systemMachineKey("linux x86_64")).toBe("common.thisComputer");
    // A system it does not recognize is a computer.
    expect(systemMachineKey("")).toBe("common.thisComputer");
    vi.unstubAllGlobals();
  });
});

describe("what this window calls each machine (PRD core-host-node-move B2, N1)", () => {
  const device = (id: string, kind: string, over: Record<string, unknown> = {}) => ({ id, kind, label: id === "core" ? null : id, ...over }) as never;
  const devices = [device("core", "local", { machine_name: "Mac mini" }), device("mbp", "remote", { label: "MacBook Pro", dials_in: true }), device("box", "remote", { label: "build-box" })];
  const t = ((key: string) => ({ "common.thisMac": "이 Mac", "common.thisPc": "이 PC" })[key] ?? key) as never;

  beforeEach(() => {
    vi.stubGlobal("navigator", { platform: "MacIntel" });
  });

  afterEach(() => {
    window.location.hash = "";
    vi.unstubAllGlobals();
  });

  it("names the core's own machine This Mac in a window on the core", () => {
    window.location.hash = "#token=t";
    expect(machineName(devices, "core", t)).toBe("이 Mac");
    expect(machineName(devices, "mbp", t)).toBe("MacBook Pro");
    expect(windowFirst(devices).map((row) => (row as { id: string }).id)).toEqual(["core", "mbp", "box"]);
  });

  it("names the node This Mac and the core's machine by its own name in a window on a node, which comes first", () => {
    window.location.hash = "#token=t&node=mbp";
    expect(machineName(devices, "mbp", t)).toBe("이 Mac");
    expect(machineName(devices, "core", t)).toBe("Mac mini");
    expect(coreMachineName(devices, t)).toBe("Mac mini");
    expect(windowFirst(devices).map((row) => (row as { id: string }).id)).toEqual(["mbp", "core", "box"]);
  });

  it("names this window's machine as its own system does, whichever machine the core runs on", () => {
    vi.stubGlobal("navigator", { platform: "Win32" });
    window.location.hash = "#token=t&node=mbp";
    expect(machineName(devices, "mbp", t)).toBe("이 PC");
    expect(machineName(devices, "core", t)).toBe("Mac mini");
  });
});

// The rows a MacBook's window reads after the core moved to the Mac mini: the
// core's own row is the mini, which keeps the label it was registered under,
// and the MacBook is a node that dialed in, registered by its host name.
describe("a window on the machine the core moved off", () => {
  const moved = [
    { id: "mini-node", kind: "local", label: "Mac mini", machine_name: "Mac-mini.local" },
    { id: "mbp", kind: "remote", label: "MacBook-Pro.local", dials_in: true },
    { id: "box", kind: "remote", label: "build-box" },
  ] as never;
  const t = ((key: string) => ({ "common.thisMac": "This Mac" })[key] ?? key) as never;
  const rest = (focused: string, devices = moved) => ({ navigator: { devices, focused_device_id: focused } }) as never;

  beforeEach(() => {
    vi.stubGlobal("navigator", { platform: "MacIntel" });
    window.location.hash = "#token=t&node=mbp";
  });

  afterEach(() => {
    window.location.hash = "";
    vi.unstubAllGlobals();
  });

  it("names the core's machine by its registered label, its own name only when it has none, and itself This Mac", () => {
    expect(machineName(moved, "mini-node", t)).toBe("Mac mini");
    expect(coreMachineName(moved, t)).toBe("Mac mini");
    expect(machineName(moved, "mbp", t)).toBe("This Mac");
    const unlabelled = [{ id: "mini-node", kind: "local", label: null, machine_name: "Mac-mini.local" }] as never;
    expect(coreMachineName(unlabelled, t)).toBe("Mac-mini.local");
  });

  it("draws no device band for its own machine, and one for the core's or any other", () => {
    expect(awayDevice(rest("mbp"))).toBeNull();
    expect((awayDevice(rest("mini-node")) as { id: string } | null)?.id).toBe("mini-node");
    expect((awayDevice(rest("box")) as { id: string } | null)?.id).toBe("box");
    // The core's own window: its machine has no band, a device does.
    window.location.hash = "#token=t";
    expect(awayDevice(rest("mini-node"))).toBeNull();
    expect((awayDevice(rest("mbp")) as { id: string } | null)?.id).toBe("mbp");
  });
});

// A Windows PC's screen that a Mac's desktop app shows (UI_BEHAVIOR.md, Settings
// and the Device rail): the page runs in the Mac's window, so the browser's
// system is the Mac's, and the PC is named as that window calls it. The page
// hears the showing window once per load, so the test loads the modules afresh.
describe("a screen another computer's window shows of this one", () => {
  const pc = [{ id: "pc", kind: "local", label: null, machine_name: "DESKTOP-7Q2" }, { id: "box", kind: "remote", label: "build-box" }] as never;
  const t = ((key: string) => ({ "common.thisMac": "이 Mac", "common.thisPc": "이 PC", "common.thisComputer": "이 컴퓨터" })[key] ?? key) as never;

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.resetModules();
  });

  it("names its own machine as the showing window does, and never by that window's system", async () => {
    vi.resetModules();
    vi.stubGlobal("navigator", { platform: "MacIntel" });
    let tell: ((viewer: { language: string; name: string | null }) => void) | null = null;
    vi.stubGlobal("hideRemote", { bridge: 1, onViewer: (listener: typeof tell) => { tell = listener; } });
    const { subscribeViewer } = await import("./remoteView");
    const fresh = await import("./screenMachine");
    const stop = subscribeViewer(() => undefined);
    // Before the window has said its name, and with no system on the row: a computer, not the viewer's Mac.
    expect(fresh.machineName(pc, "pc", t)).toBe("이 컴퓨터");
    tell!({ language: "ko", name: "window" });
    // Settings' title and its core machine group, and every other own-machine name.
    expect(fresh.machineName(pc, "pc", t)).toBe("window");
    expect(fresh.coreMachineName(pc, t)).toBe("window");
    // The row's own system does not outrank what the showing window calls it.
    const told = [{ id: "pc", kind: "local", label: null, host: { platform: "windows x86_64" } }] as never;
    expect(fresh.machineName(told, "pc", t)).toBe("window");
    // Another machine keeps its own name.
    expect(fresh.machineName(pc, "box", t)).toBe("build-box");
    stop();
  });
});
