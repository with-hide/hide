import { emptyScope } from "../../test/legacyAgentScope";
// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import type { Device, Kit, KitAgent, KitFeatureId, KitPiece } from "../snapshot";
import { useShellStore } from "../store";
import type { DispatchFn } from "../ws";
import { AgentsTab } from "./AgentsTab";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

const piece = (state: KitPiece["state"], reason: string | null = null): KitPiece => ({ state, reason, location: null });
const LABELS: Record<string, string> = { "claude-code": "Claude Code", codex: "Codex", grok: "Grok", opencode: "OpenCode", pi: "Pi", omp: "omp", antigravity: "Antigravity" };
const FEATURES: KitFeatureId[] = ["skill", "guidance", "letters", "bell", "memory", "spawn_guard", "herdr_integration", "sleep", "fork", "start", "titles"];

const agent = (id: string, over: Partial<KitAgent> = {}): KitAgent => ({
  id,
  label: LABELS[id]!,
  availability: "available",
  enabled: true,
  chosen: false,
  skill: piece("installed"),
  hook: piece("installed"),
  herdr: piece("installed"),
  doc_url: `https://docs.example.test/${id}`,
  ...over,
});
// What a Partial agent does in the fixture: the skill and Herdr's own status.
const supports = (feature: KitFeatureId) => ["skill", "herdr_integration"].includes(feature);
const partial = (id: string, over: Partial<KitAgent> = {}): KitAgent =>
  agent(id, {
    enabled: false,
    skill: piece("off"),
    hook: null,
    herdr: piece("off"),
    partial: true,
    features: FEATURES.map((feature) => ({ id: feature, supported: supports(feature) })),
    sessions: null,
    ...over,
  });

const SEVEN = [
  agent("claude-code", { sessions: 2 }),
  agent("codex", { sessions: 0 }),
  partial("grok"),
  agent("opencode", { enabled: false, sessions: null }),
  agent("pi", { availability: "not_installed" }),
  agent("omp", { availability: "not_installed" }),
  agent("antigravity", { availability: "not_installed" }),
];

const kit = (agents: KitAgent[], over: Partial<Kit> = {}): Kit => ({ unavailable: null, busy: false, components: [], agents, offers_reinstall: false, shares_account_with: null, ...over });
const device = (id: string, over: Partial<Device> = {}): Device => ({ agent_scope: emptyScope(), id, label: id === "local" ? "mini" : "Studio", kind: id === "local" ? "local" : "remote", state: id === "local" ? "local" : "ready", message: null, ssh_alias: id === "local" ? null : id, agent_count: 0, test: null, kit: kit(SEVEN), ...over });

const state = (devices: Device[]) => ({
  connection: "live" as const,
  rest: { navigator: { focused_device_id: "local", devices }, status: {}, ui_state: {} },
});

afterEach(() => {
  document.body.innerHTML = "";
});

async function mount(next: ReturnType<typeof state>) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const saved = useShellStore.getState();
  await act(async () => {
    useShellStore.setState(next as never);
    root.render(<TooltipProvider><AgentsTab actions={actions} /></TooltipProvider>);
  });
  const q = (selector: string) => container.querySelector(selector) as HTMLElement | null;
  const all = (selector: string) => [...container.querySelectorAll(selector)] as HTMLElement[];
  const click = async (element: HTMLElement | null) => { await act(async () => { element?.click(); }); };
  return { events, q, all, click, text: () => container.textContent ?? "", unmount: async () => { await act(async () => root.unmount()); useShellStore.setState(saved, true); } };
}

const sent = (events: Parameters<DispatchFn>[0][], kind: string) => events.filter((event) => event.kind === kind);

it("lists the installed agents in the supported order with a switch each, and folds the rest under Not installed with an install link (B8, B9, B12)", async () => {
  const { q, all, text, unmount } = await mount(state([device("local")]));
  expect(all("[data-agent-row]").map((row) => row.getAttribute("data-agent-row"))).toEqual([
    "local:claude-code:on",
    "local:codex:on",
    "local:grok:off",
    "local:opencode:off",
    "pi:not-installed",
    "omp:not-installed",
    "antigravity:not-installed",
  ]);
  expect(text()).toContain("Installed 4");
  expect(text()).toContain("Not installed 3");
  // Folded until opened; no subtitle on any row.
  expect(q("[data-agents-not-installed]")?.hasAttribute("open")).toBe(false);
  expect(text()).not.toMatch(/Skill and session hook|Skill only/);
  expect(all("[data-agent-install]").map((link) => link.getAttribute("href"))).toEqual([
    "https://pi.dev/",
    "https://omp.sh/docs/quickstart",
    "https://antigravity.google/docs/install?tab=cli",
  ]);
  // Docs is on each installed row (shown on hover or focus) and points at this build's page for the agent, never the kit's `doc_url`.
  expect(q('[data-agent-docs="codex"]')?.getAttribute("href")).toBe("https://learn.chatgpt.com/docs/build-skills");
  // One machine: no switch at the top.
  expect(q("[data-agents-machines]")).toBeNull();
  // Gemini CLI is not supported: a row an older helper still reports is not drawn (B1, B9).
  await unmount();
  const older = await mount(state([device("local", { kit: kit([...SEVEN, partial("gemini-cli", { label: "Gemini CLI", herdr: null }), partial("cursor", { label: "Cursor", herdr: null })]) })]));
  expect(older.all("[data-agent-row]")).toHaveLength(7);
  expect(older.text()).not.toContain("Gemini");
  expect(older.text()).not.toContain("Cursor");
  await older.unmount();
});

it("shows a status only for an agent that is on: Ready, how many sessions run, and nothing for Partial agents (B16, B19)", async () => {
  const set = [
    agent("claude-code", { sessions: 2 }),
    agent("codex", { sessions: 0 }),
    partial("grok", { sessions: 3 }),
  ];
  const { q, text, unmount } = await mount(state([device("local", { kit: kit(set) })]));
  expect(q('[data-agent-status="local:claude-code:sessions"]')?.textContent).toContain("2 sessions");
  expect(q('[data-agent-status="local:codex:ready"]')?.textContent).toContain("Ready");
  // A count is all a row says about sessions: no list to open and no pane to go to.
  expect(q('[data-agent-row="local:claude-code:on"] button:not([role="switch"])')).toBeNull();
  expect(text()).not.toContain("connected");
  // Partial: the chip only, on or off, never a count.
  expect(q('[data-agent-partial="grok"]')).not.toBeNull();
  expect(q('[data-agent-row="local:grok:off"] [data-agent-status]')).toBeNull();
  // A Full agent never wears the chip.
  expect(q('[data-agent-partial="codex"]')).toBeNull();
  await unmount();
});

it("shows a failed or removed part on its own row with Reinstall, and nowhere else (B13, B20)", async () => {
  const set = [agent("claude-code", { hook: piece("removed"), herdr: piece("failed", "herdr refused") }), agent("codex")];
  const { q, all, click, events, unmount } = await mount(state([device("local", { kit: kit(set) })]));
  const line = q('[data-agent-problem="local:claude-code"]')!;
  expect(line.textContent).toContain("Herdr integration: Failed: herdr refused");
  expect(line.textContent).toContain("Hook: Removed");
  expect(all("[data-agent-problem]")).toHaveLength(1);
  await click(q('[data-hook-reinstall="local:claude-code"]'));
  expect(sent(events, "kit_reinstall")[0]?.payload).toEqual({ device_id: "local", components: [], agents: ["claude-code"] });
  await unmount();
});

it("turns an agent on or off on the machine the list is for, and not while the machine installs (B13, B67)", async () => {
  const { q, click, events, unmount } = await mount(state([device("local"), device("studio")]));
  await click(q('[data-agent-switch="local:grok:off"]'));
  expect(sent(events, "kit_agent_set")[0]?.payload).toEqual({ device_id: "local", agent: "grok", enabled: true });
  await unmount();

  const busy = await mount(state([device("local", { kit: kit(SEVEN, { busy: true }) })]));
  expect(busy.q('[data-agent-switch="local:grok:off"]')?.hasAttribute("disabled")).toBe(true);
  await busy.unmount();
});

it("lists one machine at a time under a switch that appears only with a device, and shows an unreachable device as one line with Try again (B11)", async () => {
  const studio = device("studio", { kit: kit([agent("claude-code", { sessions: 0 }), agent("codex", { availability: "not_installed", enabled: false })]) });
  // The core's own machine is named as its system names it.
  const { q, all, click, unmount } = await mount(state([device("local", { host: { platform: "windows x86_64" } as Device["host"] }), studio]));
  expect(all("[data-agents-machine]").map((item) => item.textContent)).toEqual(["This PC", "Studio"]);
  expect(all("[data-agent-row]")).toHaveLength(7);
  await click(q('[data-agents-machine="studio"]'));
  expect(all("[data-agent-row]").map((row) => row.getAttribute("data-agent-row"))).toEqual(["studio:claude-code:on", "codex:not-installed"]);
  await unmount();

  const down = await mount(state([device("local"), { ...studio, state: "unavailable" }]));
  await click(down.q('[data-agents-machine="studio"]'));
  expect(down.q("[data-agents-unreachable]")?.textContent).toBe("Studio can't be reached right now.");
  expect(down.all("[data-agent-row]")).toHaveLength(0);
  await click(down.q('[data-agents-retry="studio"]'));
  expect(sent(down.events, "retry_connect")[0]?.payload).toEqual({ target_id: "studio" });
  await down.unmount();

  // Still connecting, the Devices tab withholds Retry and so does this tab; a device switched off there points at Devices.
  const connecting = await mount({ ...state([device("local"), { ...studio, state: "unavailable" }]), rest: { ...state([device("local"), { ...studio, state: "unavailable" }]).rest, status: { remote: [{ target_id: "studio", state: "connecting" }] } } } as never);
  await click(connecting.q('[data-agents-machine="studio"]'));
  expect(connecting.q('[data-agents-blocked="connecting:studio"]')?.textContent).toBe("Connecting to Studio…");
  expect(connecting.q("[data-agents-retry]")).toBeNull();
  expect(connecting.q("[data-agents-unreachable]")).toBeNull();
  await connecting.unmount();
  const off = await mount(state([device("local"), { ...studio, state: "disabled" }]));
  await click(off.q('[data-agents-machine="studio"]'));
  expect(off.q('[data-agents-blocked="disabled:studio"]')?.textContent).toBe("Studio is switched off in Devices.");
  expect(off.q("[data-agents-retry]")).toBeNull();
  await off.unmount();
});

it("on a window of a node, lists the window's machine first and once, beside the core's machine, and resolves each to its agents", async () => {
  window.location.hash = "#token=t&node=14ad0319-60d7-5a7c-ae5a-f71550dc8a60";
  // A moved core: its own row on the mini, and the MacBook that dials in to it, whose window this is.
  const core = device("local", { id: "b06a260a-6c14-5287-9caf-672044350d94", machine_name: "example-Mac-mini.local" });
  const node = device("14ad0319-60d7-5a7c-ae5a-f71550dc8a60", { label: "example-MacBook-Pro.local", machine_name: "example-MacBook-Pro.local", ssh_alias: null, dials_in: true, host: { platform: "macos aarch64" } as Device["host"] });
  const { q, all, click, unmount } = await mount(state([core, node]));
  expect(all("[data-agents-machine]").map((item) => item.getAttribute("data-agents-machine"))).toEqual([node.id, core.id]);
  expect(all("[data-agent-row]")[0]?.getAttribute("data-agent-row")).toBe(`${node.id}:claude-code:on`);
  expect(q("[data-agents-unread]")).toBeNull();
  await click(q(`[data-agents-machine="${core.id}"]`));
  expect(all("[data-agent-row]")[0]?.getAttribute("data-agent-row")).toBe(`${core.id}:claude-code:on`);
  await unmount();
  window.location.hash = "";
});

it("says a machine whose kit answered with no agent could not be read, with Check again, rather than checking for good", async () => {
  const failed = { id: "cli", state: "failed", reason: "another Hide was still changing this account's kit after 90 seconds", location: null } as Kit["components"][number];
  const { q, click, events, text, unmount } = await mount(state([device("local", { kit: kit([], { components: [failed] }) })]));
  expect(q("[data-agents-unread]")).toBeNull();
  expect(q('[data-agents-unlisted="local"]')?.textContent).toBe("Hide couldn't read this machine's agents.");
  const opened = sent(events, "kit_check").length;
  await click(q('[data-agents-unlisted-check="local"]'));
  expect(sent(events, "kit_check")).toHaveLength(opened + 1);
  expect(text()).not.toContain("Checking this machine's agents");
  await unmount();

  // While that read runs it says so, and before the first answer too.
  const reading = await mount(state([device("local", { kit: kit([], { components: [failed], checking: true }) })]));
  expect(reading.q('[data-agents-unread="local"]')?.textContent).toBe("Checking this machine's agents…");
  expect(reading.q("[data-agents-unlisted]")).toBeNull();
  await reading.unmount();
});

it("Check again asks the kit and shows the kit's own state: under way while it reads, the old list and one reason when it failed (B10)", async () => {
  const { q, click, events, unmount } = await mount(state([device("local", { kit: kit(SEVEN, { unavailable: "A daemon outside the package installs nothing" }) })]));
  expect(q("[data-agents-unavailable]")?.textContent).toBe("A daemon outside the package installs nothing");
  expect(q('[data-agent-row="local:claude-code:on"]')).not.toBeNull();
  expect(q("[data-agents-check]")?.getAttribute("data-agents-check")).toBe("idle");
  // The tab reads the kit once as it opens (the demand hook); the button asks again and keeps no clock of its own.
  const opened = sent(events, "kit_check").length;
  await click(q("[data-agents-check]"));
  expect(sent(events, "kit_check")).toHaveLength(opened + 1);
  expect(q("[data-agents-check]")?.getAttribute("data-agents-check")).toBe("idle");
  await unmount();

  const reading = await mount(state([device("local", { kit: kit(SEVEN, { checking: true }) })]));
  expect(reading.q("[data-agents-check]")?.getAttribute("data-agents-check")).toBe("checking");
  expect(reading.q("[data-agents-check]")?.hasAttribute("disabled")).toBe(true);
  expect(reading.q("[data-agents-check]")?.getAttribute("aria-busy")).toBe("true");
  await reading.unmount();

  const failed = await mount(state([device("local", { kit: kit(SEVEN, { check_failed: "timed_out" }) })]));
  expect(failed.q("[data-agents-check-failed]")?.textContent).toBe("Couldn't check again: it took too long");
  expect(failed.all("[data-agent-row]")).toHaveLength(7);
  await failed.unmount();
});

it("draws no Docs link from a URL a machine reports, whatever its scheme (W7)", async () => {
  const hostile = [agent("claude-code", { doc_url: "javascript:alert(1)" }), agent("codex", { doc_url: "file:///etc/passwd" }), ...SEVEN.slice(2)];
  const { all, q, unmount } = await mount(state([device("local", { kit: kit(hostile) })]));
  const hrefs = all("a[href]").map((link) => link.getAttribute("href") ?? "");
  expect(hrefs.filter((href) => !href.startsWith("https://"))).toEqual([]);
  expect(q('[data-agent-docs="claude-code"]')?.getAttribute("href")).toBe("https://code.claude.com/docs/en/skills");
  await unmount();
});

it("lists an agent that is on only by default, with no program, as not installed and never Ready (B9)", async () => {
  const set = [agent("claude-code", { availability: "not_installed", enabled: true, chosen: false, sessions: 0 }), agent("codex", { sessions: 0 }), ...SEVEN.slice(2)];
  const { all, text, q, unmount } = await mount(state([device("local", { kit: kit(set) })]));
  expect(all("[data-agent-row]").map((row) => row.getAttribute("data-agent-row"))).toContain("claude-code:not-installed");
  expect(q('[data-agent-row="local:claude-code:on"]')).toBeNull();
  expect(text()).toContain("Installed 3");
  await unmount();
  // An agent the operator switched on keeps its row and switch after its program went away.
  const kept = [agent("claude-code", { availability: "not_installed", enabled: true, chosen: true, sessions: 0 }), ...SEVEN.slice(1)];
  const again = await mount(state([device("local", { kit: kit(kept) })]));
  expect(again.q('[data-agent-row="local:claude-code:on"]')).not.toBeNull();
  expect(again.q('[data-agent-status="local:claude-code:ready"]')).toBeNull();
  await again.unmount();
});

it("keeps a recorded-on agent with no program under Installed with its switch, from the row exactly as the core serializes it (B9, D-07)", async () => {
  // `KitAgentSnapshot` as serde_json writes it, so a field the core does not send cannot be papered over by a fixture.
  const wire = (id: string, label: string, chosen: boolean): KitAgent =>
    JSON.parse(
      JSON.stringify({
        id,
        label,
        availability: "not_installed",
        enabled: true,
        chosen,
        skill: { state: "absent", reason: "`" + id + "` is not found", location: null },
        hook: { state: "absent", reason: "`" + id + "` is not found", location: null },
        herdr: { state: "absent", reason: null, location: null },
        partial: false,
        features: FEATURES.map((feature) => ({ id: feature, supported: true })),
        sessions: 0,
        doc_url: `https://docs.example.test/${id}`,
      }),
    );
  const set = [wire("claude-code", "Claude Code", false), wire("codex", "Codex", true), ...SEVEN.slice(2)];
  const { all, q, unmount } = await mount(state([device("local", { kit: kit(set) })]));
  const rows = all("[data-agent-row]").map((row) => row.getAttribute("data-agent-row"));
  expect(rows).toContain("local:codex:on");
  expect(q('[data-agent-row="local:codex:on"] [role="switch"]')).not.toBeNull();
  expect(rows).toContain("claude-code:not-installed");
  expect(q('[data-agent-row="local:claude-code:on"]')).toBeNull();
  await unmount();
});

it("opens the Partial popover from the chip and lists every feature with a mark and a word, and no screen-only line (B18, B8)", async () => {
  const { q, click, unmount } = await mount(state([device("local")]));
  await click(q('[data-agent-partial="grok"]'));
  const popover = document.body.querySelector('[data-agent-partial-popover="grok"]')!;
  const lines = [...popover.querySelectorAll("[data-agent-feature]")].map((line) => line.getAttribute("data-agent-feature"));
  expect([...lines].sort()).toEqual(FEATURES.map((feature) => `${feature}:${supports(feature) ? "yes" : "no"}`).sort());
  const groups = [...popover.querySelectorAll("[data-agent-feature-group]")];
  expect(groups.map((group) => group.getAttribute("data-agent-feature-group"))).toEqual(["herdr", "sessions", "collaboration"]);
  expect(groups.map((group) => [...group.querySelectorAll("[data-agent-feature]")].map((line) => line.getAttribute("data-agent-feature")?.split(":")[0]))).toEqual([
    ["skill", "guidance", "herdr_integration", "start"],
    ["titles", "sleep", "fork"],
    ["letters", "bell", "memory", "spawn_guard"],
  ]);
  expect(popover.querySelector('[data-agent-feature="letters:no"]')?.textContent).toContain("–Not available: Letters and Observer warnings");
  expect(popover.querySelector('[data-agent-feature="skill:yes"]')?.textContent).toContain("✓Works: Hide skill");
  // Every supported agent has Herdr's integration, so no row says its status is judged from the screen.
  expect(popover.textContent).not.toContain("from the screen");
  await unmount();
});
