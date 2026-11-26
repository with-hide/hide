import { beforeEach, describe, expect, it, vi } from "vitest";
import { clearAnswerPathChecks, findAnswerPaths, MAX_PATHS, writtenPaths } from "./answerPaths";
import { requestFileBytesWithTotal } from "./fileBytes";
import { hostBridge, type ProbedPath } from "./host";
import type { SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { clearProbeCache } from "./terminalLinkProvider";

vi.mock("./fileBytes", () => ({ requestFileBytesWithTotal: vi.fn() }));
vi.mock("./host", async (original) => ({ ...(await original<typeof import("./host")>()), hostBridge: vi.fn() }));

/** The spellings `text` writes, as the chips would cover them. */
const spelled = (text: string) => writtenPaths(text).map((one) => [text.slice(one.from, one.to), one.written]);

describe("writtenPaths", () => {
  it("reads a path from a word of prose without its sentence's punctuation or a Korean particle", () => {
    expect(spelled("보고서는 agents/runs/rc9/report.pdf, 바뀐 점은 release-notes.md에 정리했어요.")).toEqual([
      ["agents/runs/rc9/report.pdf", "agents/runs/rc9/report.pdf"],
      ["release-notes.md", "release-notes.md"],
    ]);
  });

  it("takes inline code that holds a path alone as the whole span, and keeps a fenced block and other code as code", () => {
    const text = "Open `web/src/App.tsx:12` then run `pnpm test web/src/a.ts`.\n```sh\ncat web/src/b.ts\n```\nand /tmp/crash.log";
    expect(spelled(text)).toEqual([
      ["`web/src/App.tsx:12`", "web/src/App.tsx"],
      ["/tmp/crash.log", "/tmp/crash.log"],
    ]);
  });

  it("leaves addresses and a Markdown link's target alone", () => {
    expect(spelled("See https://example.com/a.md and [the notes](docs/notes.md).")).toEqual([]);
  });
});

describe("findAnswerPaths", () => {
  const rest = {
    navigator: {
      devices: [{ id: "core", kind: "local" }, { id: "deepbox", kind: "remote" }],
      workspaces: [
        { id: "w1", device_id: "core", checkouts: [{ id: "c1", path: "/work/hide", exists: true, tabs: [{ panes: [{ id: "p1", cwd: "/work/hide/web" }] }] }] },
        { id: "w2", device_id: "deepbox", checkouts: [{ id: "c2", path: "/home/me/repo", exists: true, tabs: [{ panes: [{ id: "p2", cwd: "/home/me/repo" }] }] }] },
      ],
    },
  } as unknown as SnapshotRest;
  const files = new Set(["/work/hide/web/src/App.tsx", "/home/me/repo/notes.md"]);
  const outside: Record<string, ProbedPath> = { "/Users/me/Downloads/crash.log": { real: "/Users/me/Downloads/crash.log", kind: "file" }, "/tmp/link.md": { real: "/work/hide/docs/link.md", kind: "file" }, "/work/hide": { real: "/work/hide", kind: "directory" } };
  const probe = vi.fn(async (paths: string[]) => paths.map((path) => outside[path] ?? null));

  beforeEach(() => {
    clearAnswerPathChecks();
    clearProbeCache();
    probe.mockClear();
    useShellStore.setState({ rest });
    vi.mocked(hostBridge).mockReturnValue({ probePaths: probe } as never);
    vi.mocked(requestFileBytesWithTotal).mockReset().mockImplementation(async (path) => {
      if (!files.has(path)) throw new Error("not_found");
      return { bytes: new Uint8Array([1]), total: 10 };
    });
  });

  it("makes a project's file a project chip, reading a relative path under the session's own folder, and leaves a missing one out", async () => {
    const found = await findAnswerPaths(["src/App.tsx", "src/Gone.tsx"], { device: "core", cwd: "/work/hide/web" }, rest, "core");
    expect([...found]).toEqual([["src/App.tsx", { kind: "project", path: "/work/hide/web/src/App.tsx", device: "core", root: "/work/hide" }]]);
    // The core's own file is read without naming a device, by its first byte only.
    expect(vi.mocked(requestFileBytesWithTotal).mock.calls[0]).toEqual(["/work/hide/web/src/App.tsx", { length: 1 }, null]);
  });

  it("makes a file of this computer outside every project an outside chip, and one a symlink leads into a project that project's file", async () => {
    const found = await findAnswerPaths(["/Users/me/Downloads/crash.log", "/tmp/link.md", "/nowhere/x.log"], { device: "core", cwd: null }, rest, "core");
    expect(Object.fromEntries(found)).toEqual({
      "/Users/me/Downloads/crash.log": { kind: "outside", path: "/Users/me/Downloads/crash.log" },
      "/tmp/link.md": { kind: "project", path: "/work/hide/docs/link.md", device: "core", root: "/work/hide" },
    });
  });

  it("reads a connected device's project file on that device, and leaves its outside paths as words, since this computer cannot show them", async () => {
    const found = await findAnswerPaths(["notes.md", "/Users/me/Downloads/crash.log"], { device: "deepbox", cwd: "/home/me/repo" }, rest, "core");
    expect([...found]).toEqual([["notes.md", { kind: "project", path: "/home/me/repo/notes.md", device: "deepbox", root: "/home/me/repo" }]]);
    expect(vi.mocked(requestFileBytesWithTotal).mock.calls[0]).toEqual(["/home/me/repo/notes.md", { length: 1 }, { device: "deepbox", root: "/home/me/repo" }]);
    expect(probe).not.toHaveBeenCalled();
  });

  it("looks up at most 32 paths in one answer", async () => {
    const many = Array.from({ length: MAX_PATHS + 8 }, (_, index) => `/nowhere/${index}.log`);
    await findAnswerPaths(many, { device: "core", cwd: null }, rest, "core");
    expect(probe.mock.calls.flatMap(([paths]) => paths)).toHaveLength(MAX_PATHS);
  });
});
