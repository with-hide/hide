// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import type { DeskOutput } from "./desk";
import { DeskOutputView } from "./DeskOutputView";
import { clientI18n } from "./i18n/translator";
import { useShellStore } from "./store";
import { TooltipProvider } from "./components/ui/tooltip";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });

const reads = vi.hoisted(() => ({ calls: [] as { path: string; source: unknown }[], outcome: [] as (Uint8Array | Error)[] }));
vi.mock("./fileBytes", () => ({
  requestFileBytes: async (path: string, _range: unknown, source: unknown) => {
    reads.calls.push({ path, source });
    const next = reads.outcome.shift() ?? new Error("read_failed");
    if (next instanceof Error) throw next;
    return next;
  },
}));

const output = (extra: Partial<DeskOutput> = {}): DeskOutput => ({ id: "view-1", kind: "doc", label: "notes.md", thumb: null, seen: true, pr_state: null, device_id: "mac", checkout: "/work/repo", path: "/work/repo/notes.md", url: null, ...extra });

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
let saved: ReturnType<typeof useShellStore.getState>;
const actions = createActions(() => true);
const show = async (shown: DeskOutput) => act(async () => root.render(<TooltipProvider><DeskOutputView output={shown} viewKey="k" actions={actions} /></TooltipProvider>));

beforeEach(async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  saved = useShellStore.getState();
  reads.calls = [];
  reads.outcome = [];
  useShellStore.setState({ rest: { navigator: { devices: [{ id: "mac", kind: "local" }, { id: "deepbox", kind: "remote" }] } } as never, diagnostics: [] });
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
});

describe("a file output that cannot be read (PRD desk-2-ui B28)", () => {
  it("says why in plain words with 다시 시도 inside its tab, keeps the code in the log, and reads again on the press", async () => {
    reads.outcome = [new Error("not_found"), new TextEncoder().encode("# 안녕")];
    await show(output());
    const failed = container.querySelector<HTMLElement>("[data-desk-output-failed]")!;
    expect(failed.textContent).toContain("이 결과물을 읽을 수 없어요");
    expect(failed.textContent).not.toContain("not_found");
    expect(failed.textContent).toContain("다시 시도");
    expect(useShellStore.getState().diagnostics.join("\n")).toContain("reason=not_found");

    await act(async () => container.querySelector<HTMLElement>("[data-desk-output-retry]")!.click());
    expect(reads.calls).toHaveLength(2);
    expect(container.querySelector("[data-desk-output-failed]")).toBeNull();
    expect(container.querySelector("[data-desk-output-document]")?.textContent).toContain("안녕");
  });

  it("does the same for an image, whose own viewer would have drawn only a code", async () => {
    reads.outcome = [new Error("not_found")];
    await show(output({ kind: "image", label: "shot.png", path: "/work/repo/shot.png" }));
    expect(container.querySelector("[data-desk-output-retry]")).not.toBeNull();
  });
});

describe("a device session's output (B27)", () => {
  it("is read through the device and checkout its View names, never through this machine's boundary", async () => {
    reads.outcome = [new TextEncoder().encode("hi")];
    await show(output({ device_id: "deepbox", checkout: "/home/me/repo", path: "/home/me/repo/notes.md" }));
    expect(reads.calls).toEqual([{ path: "/home/me/repo/notes.md", source: { device: "deepbox", root: "/home/me/repo" } }]);
  });

  it("reads this machine's own session's file with no source", async () => {
    reads.outcome = [new TextEncoder().encode("hi")];
    await show(output());
    expect(reads.calls).toEqual([{ path: "/work/repo/notes.md", source: null }]);
  });
});
