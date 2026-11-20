// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { TooltipProvider } from "../components/ui/tooltip";
import { clientI18n } from "../i18n/translator";
import type { McpServer, StoreReport } from "../snapshot";
import { McpFields } from "./StoreDialogs";

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
