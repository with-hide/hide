// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { clientI18n } from "../i18n/translator";
import type { SnapshotRest, StoreReport } from "../snapshot";
import { useShellStore } from "../store";
import { SkillsTab } from "./SkillsTab";

// Settings › 스킬·MCP before and after a machine's first store report: three
// quiet rows while it is read, the empty state once it holds nothing, and
// the table once it holds an item (PRD skill-mcp-manager, layout C).
afterEach(() => {
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

const agents = [{ id: "claude-code", label: "Claude Code", availability: "available", enabled: true, chosen: false, skill: { state: "installed", why: null, location: null }, hook: null, doc_url: "https://example.test" }];

async function render(store: StoreReport | undefined, send: (event: unknown) => void = () => {}) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  await clientI18n.changeLanguage("en");
  const rest = { status: { remote: [] }, navigator: { devices: [{ id: "mac", kind: "local", kit: { agents, components: [], store } }] } } as unknown as SnapshotRest;
  useShellStore.setState({ rest, connection: "live" });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><SkillsTab actions={createActions(send as never)} /></TooltipProvider>));
  return { container, unmount: () => act(async () => root.unmount()) };
}

const report = (items: StoreReport["items"]): StoreReport => ({ columns: { skill: [{ id: "claude", agents: ["claude-code"] }] }, items, unmanaged: [], trash: [], previews: [], diffs: [] }) as StoreReport;

it("reads as three quiet rows until the machine's first report, then as empty when it holds nothing", async () => {
  const reading = await render(undefined);
  expect(reading.container.querySelector("[data-store-loading]")).not.toBeNull();
  expect(reading.container.querySelector("[data-store-empty]")).toBeNull();
  await reading.unmount();

  const empty = await render(report([]));
  expect(empty.container.querySelector("[data-store-loading]")).toBeNull();
  expect(empty.container.querySelector('[data-store-empty="mac"]')?.textContent).toContain("Nothing on");
  await empty.unmount();
});

it("draws a held item as a row of the kind's table that unfolds below itself", async () => {
  const item = { id: "skill:archify", kind: "skill", name: "archify", source: { kind: "folder", path: "/skills/archify" }, version: { kind: "folder" }, on: true, cells: [{ column: "claude", state: "on" }] };
  const { container, unmount } = await render(report([item] as StoreReport["items"]));
  try {
    expect(container.querySelector('[data-store-cell="skill:archify:claude"]')?.getAttribute("data-store-cell-state")).toBe("on");
    expect(container.querySelector('[data-store-detail="skill:archify"]')).toBeNull();
    await act(async () => container.querySelector<HTMLElement>('[data-store-row="skill:archify"] td')!.click());
    expect(container.querySelector('[data-store-detail="skill:archify"]')?.textContent).toContain("/skills/archify");
  } finally {
    await unmount();
  }
});

it("draws a GitHub skill's tile from the picture the store keeps, never from the web, and the GitHub mark without one", async () => {
  const github = { id: "skill:archify", kind: "skill", name: "archify", owner: "acme", source: { kind: "git", url: "https://github.com/acme/kit.git", path: "archify" }, version: { kind: "tag", value: "v1" }, on: true, cells: [{ column: "claude", state: "on" }] };
  const picture = "data:image/png;base64,iVBORw0KGgo=";
  const kept = await render({ ...report([github] as StoreReport["items"]), avatars: { acme: picture } });
  try {
    expect(kept.container.querySelector('[data-store-avatar="acme"]')?.getAttribute("src")).toBe(picture);
    expect([...kept.container.querySelectorAll("img")].some((img) => /^https?:/.test(img.getAttribute("src") ?? ""))).toBe(false);
  } finally {
    await kept.unmount();
  }
  const bare = await render(report([github] as StoreReport["items"]));
  try {
    expect(bare.container.querySelector("[data-store-avatar]")).toBeNull();
    expect(bare.container.querySelector('[data-store-tile="github"]')).not.toBeNull();
  } finally {
    await bare.unmount();
  }
});

// Taking over an agent's own copy of a server shows that copy's command, as
// its place holds it, not the store's own server of the name (B20); Cancel
// sends nothing, Apply takes in that place. A server Hide cannot read offers
// no Apply.
const sent = (events: unknown[]) => events.map((event) => JSON.stringify(event)).filter((text) => text.includes('"op":"adopt"'));

it("confirms taking over an agent's own server with that server's command, and Cancel sends nothing", async () => {
  const mine = { name: "github", reach: { kind: "local", command: "npx", args: ["-y", "@modelcontextprotocol/server-github"], env: ["GITHUB_TOKEN"] } };
  const theirs = { name: "github", reach: { kind: "local", command: "docker", args: ["run", "-i", "ghcr.io/github/github-mcp-server"], env: ["GITHUB_TOKEN"] } };
  const item = { id: "mcp:github", kind: "mcp", name: "github", source: { kind: "pasted" }, version: { kind: "folder" }, on: true, server: mine, cells: [{ column: "claude", state: "hand_written", place: "claude-file", path: "/home/al/.claude.json", server: theirs }] };
  const events: unknown[] = [];
  const { container, unmount } = await render({ ...report([]), columns: { mcp: [{ id: "claude", agents: ["claude-code"] }] }, items: [item] } as unknown as StoreReport, (event) => events.push(event));
  const press = (selector: string) => act(async () => document.querySelector<HTMLElement>(selector)!.click());
  try {
    await act(async () => container.querySelector<HTMLElement>('[data-store-row="mcp:github"] td')!.click());
    await press('[data-store-take-over="mcp:github:claude"]');
    const dialog = () => document.querySelector('[data-store-dialog="adopt"]')?.textContent ?? "";
    expect(dialog()).toContain("docker run -i ghcr.io/github/github-mcp-server");
    expect(dialog()).toContain("GITHUB_TOKEN");
    expect(dialog()).not.toContain("npx");
    await press('[data-store-dialog="adopt"] [data-store-cancel]');
    expect(document.querySelector('[data-store-dialog="adopt"]')).toBeNull();
    expect(sent(events)).toEqual([]);
    await press('[data-store-take-over="mcp:github:claude"]');
    await press('[data-store-dialog="adopt"] [data-store-mcp-add]');
    expect(sent(events)).toHaveLength(1);
    expect(JSON.parse(sent(events)[0]!).payload.changes).toEqual([{ op: "adopt", kind: "mcp", name: "github", from: "claude-file" }]);
  } finally {
    await unmount();
  }
});

it("offers no Apply for an agent's server Hide cannot read", async () => {
  const unmanaged = [{ kind: "mcp", name: "broken", same: true, copies: [{ place: "claude-file", agents: ["claude-code"], path: "/home/al/.claude.json", digest: "", refused: null, server: null }] }];
  const events: unknown[] = [];
  const { container, unmount } = await render({ ...report([]), columns: { mcp: [{ id: "claude", agents: ["claude-code"] }] }, unmanaged } as unknown as StoreReport, (event) => events.push(event));
  try {
    await act(async () => container.querySelector<HTMLElement>('[data-store-adopt="mcp:broken"]')!.click());
    expect(document.querySelector('[data-store-dialog="adopt"] [data-store-adopt-unreadable]')).not.toBeNull();
    expect(document.querySelector('[data-store-dialog="adopt"] [data-store-install]')).toBeNull();
    expect(document.querySelector('[data-store-dialog="adopt"] [data-store-mcp-add]')).toBeNull();
  } finally {
    await unmount();
  }
});
