import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { BotTileView, RailTileView, type Tile } from "./components/device-rail";
import { botLook, type BotModeKind } from "./bots";
import { TooltipProvider } from "./components/ui/tooltip";
import { sidebarBody, type TileCounts } from "./devices";
import type { SnapshotRest } from "./snapshot";

const LOCAL = { id: "local", label: "This Mac", kind: "local", state: "local", message: null };
const MINI = { id: "mini", label: "mini", kind: "remote", state: "ready", message: null };
const OFF = { id: "build-box", label: "연구실 빌드 서버 자동화 장비", kind: "remote", state: "unavailable", message: null };

function world(devices: unknown[], front = "local", connected = ["mini"]): SnapshotRest {
  return {
    navigator: { focused_device_id: front, devices },
    status: { remote: [MINI, OFF].map((device) => ({ target_id: device.id, state: connected.includes(device.id) ? "connected" : "stale", session: null })) },
  } as unknown as SnapshotRest;
}

const tile = (html: string) => html.match(/<button[^>]*data-rail-tile[^>]*>/)?.[0] ?? "";

const NONE: TileCounts = { needs_you: 0, done: 0 };

function view(props: { tile: Tile; selected?: boolean; counts?: TileCounts; connected?: boolean }) {
  return renderToStaticMarkup(
    <TooltipProvider>
      <RailTileView selected={false} counts={NONE} connected onSelect={() => undefined} {...props} />
    </TooltipProvider>,
  );
}

/** Each mark drawn, in source order, with its text: the Done dot has none. */
const marks = (html: string) => [...html.matchAll(/data-rail-badge="(\w+)"[^>]*>([^<]*)</g)].map((match) => [match[1], match[2]]);

describe("which list fills the sidebar (quick device-rail-badges B3)", () => {
  it("is the Projects list for a connected device in front, one device or many", () => {
    expect(sidebarBody(world([LOCAL]))).toBe("projects");
    expect(sidebarBody(world([LOCAL, MINI], "mini"))).toBe("projects");
    expect(sidebarBody(world([LOCAL, MINI]))).toBe("projects");
  });

  it("is the reconnect view for a front device that is not connected, even when it is the only remote one", () => {
    expect(sidebarBody(world([LOCAL, OFF], "build-box"))).toBe("disconnected");
    expect(sidebarBody(world([LOCAL, MINI], "mini", []))).toBe("disconnected");
  });
});

describe("a rail tile (quick device-rail-slack)", () => {
  const local: Tile = { id: "local", label: "This Mac", icon: "local" };
  const mini: Tile = { id: "mini", label: "Mac mini", icon: "remote" };
  const off: Tile = { id: "build-box", label: OFF.label, icon: "remote" };

  it("is a focusable button whose name carries the device and each count it marks (B4)", () => {
    const html = view({ tile: mini, counts: { needs_you: 2, done: 1 } });
    expect(tile(html)).toContain('aria-label="Mac mini, Needs You 2, Unread 1"');
    expect(html.startsWith("<button")).toBe(true);
    expect(html).not.toContain("tabindex");
  });

  // PRD wsl-device B5, D-20: a WSL distribution's tile draws its name's letters and says where it is.
  it("letters a WSL distribution's tile from its name and names it with where it is, never Remote", () => {
    const wsl: Tile = { id: "wsl-ubuntu-1a2b3c4d", label: "WSL Ubuntu", icon: "remote", place: "This PC" };
    const html = view({ tile: wsl, counts: { needs_you: 1, done: 0 } });
    expect(html).toMatch(/data-rail-glyph="true"[^>]*>WU</);
    expect(tile(html)).toContain('aria-label="WSL Ubuntu · This PC, Needs You 1"');
    expect(html).not.toContain("Remote");
  });

  it("draws no name under the tile: This Mac is the laptop and a device its monogram", () => {
    expect(view({ tile: local })).toContain("lucide-laptop");
    const remote = view({ tile: mini });
    expect(remote).toMatch(/data-rail-glyph="true"[^>]*>Mm</);
    expect(remote).not.toContain("data-rail-label");
    expect(remote).not.toContain(">Mac mini<");
  });

  it("shows one mark, the most urgent: the Needs You count over an unseen Done dot", () => {
    const html = view({ tile: mini, counts: { needs_you: 2, done: 8 } });
    expect(marks(html)).toEqual([["needs_you", "2"]]);
    expect(html).toContain("bg-warning");
    expect(html).not.toContain("bg-success");
    expect(html).toContain("text-status-foreground");
    // Done alone is a dot with no number; the hint still names both counts.
    expect(marks(view({ tile: mini, counts: { needs_you: 0, done: 3 } }))).toEqual([["done", ""]]);
    expect(tile(html)).toContain('aria-label="Mac mini, Needs You 2, Unread 8"');
  });

  it("draws a count of ten or more as `9+`, and no mark at all when both counts are zero (B4)", () => {
    const busy = view({ tile: mini, counts: { needs_you: 12, done: 0 } });
    expect(marks(busy)).toEqual([["needs_you", "9+"]]);
    expect(tile(busy)).toContain("Needs You 12");
    expect(view({ tile: mini })).not.toContain("data-rail-badge");
  });

  it("rings the tile only while selected, with no bar beside it", () => {
    const quiet = tile(view({ tile: mini }));
    expect(quiet).not.toContain("ring-foreground");
    const chosen = tile(view({ tile: mini, selected: true }));
    expect(chosen).toContain("ring-foreground");
    expect(chosen).toContain('aria-pressed="true"');
    expect(view({ tile: mini, selected: true })).not.toContain("data-rail-bar");
  });

  it("dims a disconnected device's glyph with a cross, no mark, and reads Not connected (B8)", () => {
    // Counts a stale session still carries are not drawn.
    const html = view({ tile: off, connected: false, counts: { needs_you: 1, done: 1 } });
    expect(html).toContain("data-rail-off");
    expect(html).not.toContain("data-rail-badge");
    expect(html).toMatch(/opacity-\(--opacity-dimmed\)[^>]*data-rail-glyph|data-rail-glyph[^>]*opacity-\(--opacity-dimmed\)/);
    expect(tile(html)).toContain('aria-label="연구실 빌드 서버 자동화 장비, Not connected"');
  });
});

describe("a bot's tile (PRD hide-external-control D-50, B39, B41)", () => {
  const bot = (turn: number, { selected = false, mode = "own" as BotModeKind } = {}) => renderToStaticMarkup(
    <TooltipProvider>
      <BotTileView name="nightly" turn={turn} connection="c0ffee00c0ffee00" mode={mode} selected={selected} onSelect={() => undefined} />
    </TooltipProvider>,
  );
  const button = (html: string) => html.match(/<button[^>]*data-rail-tool[^>]*>/)?.[0] ?? "";

  it("draws the owl its name chooses, names it a bot and its mode, and carries what waits on it in a grey number", () => {
    const html = bot(3);
    const look = botLook("nightly");
    expect(html).toContain(`data-bot-face="${look.shape}:${look.color}"`);
    expect(button(html)).toContain('aria-label="nightly, Bot, Own space, Bot&#x27;s turn 3"');
    expect(marks(html)).toEqual([["tool_turn", "3"]]);
    expect(html).toContain("bg-muted-foreground");
    expect(html).not.toContain("bg-warning");
  });

  it("reads 9+ from ten and draws no number when nothing waits on the bot", () => {
    expect(marks(bot(10))).toEqual([["tool_turn", "9+"]]);
    expect(marks(bot(0))).toEqual([]);
    expect(button(bot(0))).toContain('aria-label="nightly, Bot, Own space"');
  });

  it("rings the owl in lime only while the bot works in all of Hide, which its name says", () => {
    expect(bot(0)).not.toContain("ring-primary");
    const all = bot(0, { mode: "all" });
    expect(all).toMatch(/data-bot-mode="all"[^>]*ring-primary|ring-primary[^>]*data-bot-mode="all"/);
    expect(button(all)).toContain('aria-label="nightly, Bot, Full"');
  });

  it("rings as selected while its cell is in front", () => {
    expect(button(bot(0, { selected: true }))).toContain('aria-pressed="true"');
    expect(button(bot(0, { selected: true }))).toContain("ring-2");
  });
});
