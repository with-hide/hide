// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { TooltipProvider } from "../components/ui/tooltip";
import { clientI18n } from "../i18n/translator";
import type { Device, McpServer, SnapshotRest, StoreChange, StorePreview, StoreReport } from "../snapshot";
import { useShellStore } from "../store";
import { AdoptDialog, McpFields } from "./StoreDialogs";

// MCP 추가's fields keep the command and each argument as the paste parser
// read them: Next hands on the same words, an argument with a space in it
// stays one argument, and an empty argument stays in its place.
afterEach(() => {
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

const report = { columns: {}, items: [], unmanaged: [], trash: [], previews: [], diffs: [] } as unknown as StoreReport;

async function fields(server: McpServer) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  await clientI18n.changeLanguage("en");
  const next = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><McpFields report={report} server={server} pasted={null} onClose={() => {}} onNext={next} /></TooltipProvider>));
  const press = (selector: string) => act(async () => (document.querySelector(selector) as HTMLElement).click());
  return { next, press, unmount: () => act(async () => root.unmount()) };
}

it("hands on a command and arguments with spaces in them as they were read", async () => {
  const server: McpServer = { name: "files", reach: { kind: "local", command: "C:\\Program Files\\nodejs\\node.exe", args: ["/folder with spaces/server.js", "--root", "D:\\My Skills"], env: [] } };
  const shown = await fields(server);
  try {
    expect((document.querySelector("[data-store-mcp-command]") as HTMLInputElement).value).toBe("C:\\Program Files\\nodejs\\node.exe");
    expect([...document.querySelectorAll<HTMLInputElement>("[data-store-mcp-arg]")].map((input) => input.value)).toEqual(["/folder with spaces/server.js", "--root", "D:\\My Skills"]);
    await shown.press("[data-store-mcp-next]");
    expect(shown.next).toHaveBeenCalledWith(server, {});
  } finally {
    await shown.unmount();
  }
});

it("takes an argument out and an empty one added is left out", async () => {
  const shown = await fields({ name: "github", reach: { kind: "local", command: "npx", args: ["-y", "@modelcontextprotocol/server-github"], env: [] } });
  try {
    await shown.press('[data-store-mcp-arg-remove="0"]');
    await shown.press("[data-store-mcp-arg-add]");
    expect(document.querySelectorAll("[data-store-mcp-arg]")).toHaveLength(2);
    await shown.press("[data-store-mcp-next]");
    expect(shown.next).toHaveBeenCalledWith({ name: "github", reach: { kind: "local", command: "npx", args: ["@modelcontextprotocol/server-github"], env: [] } }, {});
  } finally {
    await shown.unmount();
  }
});

it("keeps an empty argument the server was read with, and leaves out only an added row left empty", async () => {
  const server: McpServer = { name: "files", reach: { kind: "local", command: "node", args: ["server.js", "--prefix", "", "--stdio"], env: [] } };
  const shown = await fields(server);
  try {
    expect(document.querySelectorAll("[data-store-mcp-arg]")).toHaveLength(4);
    await shown.press("[data-store-mcp-arg-add]");
    await shown.press("[data-store-mcp-next]");
    expect(shown.next).toHaveBeenCalledWith(server, {});
  } finally {
    await shown.unmount();
  }
});

// [모두에 적용]'s confirm shows the reading its own request made: another
// pane's read that lands later, newer and of another folder, never replaces
// it, and Apply takes in the copy the confirm showed.
const folderPreview = (id: string, path: string, name: string, at: number): StorePreview => ({
  id,
  source: { kind: "folder", path },
  skills: [{ name, path: name, digest: `digest-${id}`, taken: false }],
  agents: ["claude-code"],
  at_unix_ms: at,
});

it("keeps showing its own reading when another pane's newer read lands, and Apply adopts what it showed", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  await clientI18n.changeLanguage("en");
  const sent: { request: string; changes: StoreChange[] }[] = [];
  const actions = { changeStore: (_device: string, request: string, changes: StoreChange[]) => sent.push({ request, changes }) } as never;
  const answered = (request: string, made: string) => useShellStore.setState({ rest: { status: { store_answers: [{ request_id: request, device_id: "mac", state: "done", outcomes: [{ change: sent[0]!.changes[0]!, made }] }] } } as unknown as SnapshotRest });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const shown = new Set(["claude-code"]);
  const draw = (previews: StorePreview[]) =>
    act(async () => root.render(<TooltipProvider><AdoptDialog device={{ id: "mac" } as Device} target={{ kind: "skill", name: "writer", from: "claude", path: "/skills/writer" }} report={{ ...report, previews }} shown={shown} actions={actions} onClose={() => {}} /></TooltipProvider>));
  try {
    await draw([]);
    expect(sent[0]?.changes).toEqual([{ op: "inspect", source: { kind: "folder", path: "/skills/writer" } }]);
    const own = folderPreview("own", "/skills/writer", "writer", 1);
    const other = folderPreview("other", "/elsewhere/reader", "reader", 2);
    await act(async () => answered(sent[0]!.request, "own"));
    await draw([own, other]);
    const dialog = () => document.querySelector('[data-store-dialog="adopt"]')!.textContent ?? "";
    expect(dialog()).toContain("/skills/writer");
    expect(dialog()).toContain("Skillswriter");
    expect(dialog()).not.toContain("Files");
    expect(dialog()).not.toContain("▲");
    expect(dialog()).not.toContain("reader");
    await draw([own, other, folderPreview("later", "/elsewhere/later", "later", 3)]);
    expect(dialog()).toContain("/skills/writer");
    expect(dialog()).not.toContain("later");
    await act(async () => (document.querySelector("[data-store-install]") as HTMLElement).click());
    expect(sent[1]?.changes).toEqual([{ op: "adopt", kind: "skill", name: "writer", from: "claude", confirmed: { kind: "skill", digest: "digest-own" } }]);
  } finally {
    await act(async () => root.unmount());
  }
});

it("ends in a refusal, not a reading that never ends, when its read answered without naming what it made", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  await clientI18n.changeLanguage("en");
  const sent: { request: string; changes: StoreChange[] }[] = [];
  const actions = { changeStore: (_device: string, request: string, changes: StoreChange[]) => sent.push({ request, changes }) } as never;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const draw = (previews: StorePreview[]) =>
    act(async () => root.render(<TooltipProvider><AdoptDialog device={{ id: "mac" } as Device} target={{ kind: "skill", name: "writer", path: "/skills/writer" }} report={{ ...report, previews }} shown={new Set(["claude-code"])} actions={actions} onClose={() => {}} /></TooltipProvider>));
  try {
    await draw([]);
    await act(async () => useShellStore.setState({ rest: { status: { store_answers: [{ request_id: sent[0]!.request, device_id: "mac", state: "done", outcomes: [{ change: sent[0]!.changes[0]! }] }] } } as unknown as SnapshotRest }));
    await draw([folderPreview("other", "/elsewhere/reader", "reader", 2)]);
    expect(document.querySelector('[data-store-adopt-refused="not_found"]')).not.toBeNull();
    expect(document.querySelector('[data-store-dialog="adopt"]')!.textContent).not.toContain("reader");
  } finally {
    await act(async () => root.unmount());
  }
});
