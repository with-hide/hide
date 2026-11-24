import { describe, expect, it, vi } from "vitest";
import { REMOTE_VIEWER_CHANNEL } from "../channel";

/** What the preload exposed to the page, and the main process's channels it listens to. */
const world = vi.hoisted(() => ({ api: null as null | { onViewer(listener: (viewer: unknown) => void): void }, channels: new Map<string, (event: unknown, value: unknown) => void>() }));
vi.mock("electron", () => ({
  contextBridge: { exposeInMainWorld: (_name: string, api: typeof world.api) => (world.api = api) },
  ipcRenderer: {
    on: (channel: string, listener: (event: unknown, value: unknown) => void) => world.channels.set(channel, listener),
    removeAllListeners: () => undefined,
    send: () => undefined,
    invoke: () => Promise.resolve(),
  },
}));

describe("the remote screen's preload", () => {
  // The main process tells a page the showing window's name once it loaded,
  // which can be before the page's first listener (the operator's report on
  // candidate 5: Settings in a computer's screen read This Mac).
  it("hands the window's name told before the page listened to its later listener, then each change", async () => {
    await import("./remote");
    const tell = (viewer: unknown) => world.channels.get(REMOTE_VIEWER_CHANNEL)?.({}, viewer);
    tell({ language: "ko", name: "Windows PC" });

    const heard: unknown[] = [];
    world.api?.onViewer((viewer) => heard.push(viewer));
    expect(heard).toEqual([{ language: "ko", name: "Windows PC" }]);

    tell({ language: "en", name: "Office PC" });
    tell({ language: "en", name: 7 });
    expect(heard).toEqual([
      { language: "ko", name: "Windows PC" },
      { language: "en", name: "Office PC" },
    ]);
  });
});
