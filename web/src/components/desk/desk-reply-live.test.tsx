// @vitest-environment jsdom
// While an answer streams, a chunk that finishes no line swaps none of its parts: the Markdown's parts are
// configured again only when a chip changes, so a chunk never redraws every chip of the answer.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { emptyScope } from "../../../test/legacyAgentScope";
import type { DeskMention } from "../../desk";
import { markdownLive } from "../../editor/markdownLivePlugin";
import { configureFileBytes, receiveBytes, receiveBytesError } from "../../fileBytes";
import type { SnapshotRest } from "../../snapshot";
import { useShellStore } from "../../store";
import { TooltipProvider } from "../ui/tooltip";
import { DeskReply } from "./desk-chat";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
vi.mock("../../editor/markdownLivePlugin", async (real) => {
  const plugin = await real<typeof import("../../editor/markdownLivePlugin")>();
  return { ...plugin, markdownLive: vi.fn(plugin.markdownLive) };
});

const rest = {
  navigator: {
    focused_device_id: "local", focused_checkout_id: null, focused_workspace_id: null,
    devices: [{ id: "local", label: "This Mac", kind: "local", state: "local" }],
    workspaces: [{ id: "w-live", label: "live", device_id: "local", agent_scope: emptyScope(), checkouts: [
      { id: "c-live", workspace_id: "w-live", path: "/work/live", label: "main", tabs: [], active_tab_id: null, strip: [] },
    ] }],
  },
} as unknown as SnapshotRest;

let mounted: ReturnType<typeof createRoot>[] = [];
let saved: ReturnType<typeof useShellStore.getState>;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  saved = useShellStore.getState();
  useShellStore.setState({ rest });
  // A project file is there when its first byte reads; any other path names nothing.
  configureFileBytes((event) => {
    const read = event.payload as Record<string, unknown>;
    queueMicrotask(() => {
      if (read.path !== "/work/live/notes.md") return receiveBytesError({ request_id: read.request_id as string, reason: "not_found" });
      const header = new TextEncoder().encode(JSON.stringify({ type: "file_bytes", request_id: read.request_id, path: read.path, offset: 0, total: 1, eof: true }));
      const frame = new Uint8Array(4 + header.length + 1);
      new DataView(frame.buffer).setUint32(0, header.length, false);
      frame.set(header, 4);
      frame[4 + header.length] = 35;
      receiveBytes(frame.buffer);
    });
  });
});
afterEach(async () => {
  for (const root of mounted) await act(async () => root.unmount());
  mounted = [];
  document.body.innerHTML = "";
  useShellStore.setState(saved, true);
  vi.unstubAllGlobals();
});

const mentions: DeskMention[] = [{ kind: "session", id: "w1:p2", name: "fix login", mark: null, agent_kind: null }];
const onFile = () => undefined;

/** How many times the answer's Markdown parts are built again over four chunks that finish no line, once `ready` holds. */
async function rebuilds(first: string, withFile: boolean, ready: (host: HTMLElement) => boolean): Promise<number> {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  mounted.push(root);
  let text = first;
  const draw = async () => {
    await act(async () => root.render(<TooltipProvider><DeskReply text={text} live mentions={mentions} onFile={withFile ? onFile : undefined} /></TooltipProvider>));
    await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
  };
  await draw();
  await vi.waitFor(() => expect(ready(host)).toBe(true));
  const before = vi.mocked(markdownLive).mock.calls.length;
  for (const step of [" 줄", "을", " 씁니다", " 계속"]) {
    text += step;
    await draw();
  }
  return vi.mocked(markdownLive).mock.calls.length - before;
}

const sessionChip = (host: HTMLElement) => host.querySelector("[data-desk-session-chip]") !== null;

it("swaps no part per chunk of a live answer whose finished lines name no path", async () => {
  expect(await rebuilds("@session(w1:p2 fix login) 가 고쳤고\n다음", true, sessionChip)).toBe(0);
});

it("swaps no part per chunk of a live answer whose finished line names a path that is no file", async () => {
  expect(await rebuilds("@session(w1:p2 fix login) 가 /work/live/gone.md 를 고쳤고\n다음", true, sessionChip)).toBe(0);
});

it("swaps no part per chunk of a live answer whose finished line names a project file, drawn as its chip", async () => {
  const fileChip = (host: HTMLElement) => sessionChip(host) && host.querySelector("[data-desk-file-chip='project']") !== null;
  expect(await rebuilds("@session(w1:p2 fix login) 가 /work/live/notes.md 를 고쳤고\n다음", true, fileChip)).toBe(0);
});
