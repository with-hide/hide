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
  // An open select scrolls its chosen option into view, which jsdom does not draw.
  Element.prototype.scrollIntoView = () => {};
});

const piece = (state: KitPiece["state"], cause: string | null = null): KitPiece => ({ state, why: cause ? { cause } : null, location: null });
const LABELS: Record<string, string> = { "claude-code": "Claude Code", codex: "Codex", grok: "Grok", opencode: "OpenCode", pi: "Pi", omp: "omp", antigravity: "Antigravity" };
const FEATURES: KitFeatureId[] = ["skill", "guidance", "letters", "bell", "memory", "sleep", "fork", "start", "titles"];

const agent = (id: string, over: Partial<KitAgent> = {}): KitAgent => ({
  id,
  label: LABELS[id]!,
  availability: "available",
  enabled: true,
  chosen: false,
  skill: piece("installed"),
  hook: piece("installed"),
  doc_url: `https://docs.example.test/${id}`,
  ...over,
});
// What a Partial agent does in the fixture: the skill alone.
const supports = (feature: KitFeatureId) => feature === "skill";
const partial = (id: string, over: Partial<KitAgent> = {}): KitAgent =>
  agent(id, {
    enabled: false,
    skill: piece("off"),
    hook: null,
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

/** The text a sighted operator reads: what is not hidden from view. */
const visible = (element: Element): string =>
  [...element.childNodes].map((node) => (node.nodeType === Node.TEXT_NODE ? node.textContent ?? "" : node instanceof HTMLElement && node.hidden ? "" : node instanceof Element ? visible(node) : "")).join("");

/** Opens a select from the keyboard, as Radix takes it in jsdom, and reads each option as it is drawn. */
async function openMenu(trigger: HTMLElement): Promise<string[]> {
  trigger.focus();
  await act(async () => { trigger.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
  return [...document.querySelectorAll<HTMLElement>("[role=option]")].map((option) => option.textContent ?? "");
}

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
  const older = await mount(state([device("local", { kit: kit([...SEVEN, partial("gemini-cli", { label: "Gemini CLI" }), partial("cursor", { label: "Cursor" })]) })]));
  expect(older.all("[data-agent-row]")).toHaveLength(7);
  expect(older.text()).not.toContain("Gemini");
  expect(older.text()).not.toContain("Cursor");
  await older.unmount();
});

it("shows only how many sessions run for an agent that is on, nothing for a healthy one with none, and nothing for Partial agents (B16, B19)", async () => {
  const set = [
    agent("claude-code", { sessions: 2 }),
    agent("codex", { sessions: 0 }),
    partial("grok", { sessions: 3 }),
  ];
  const { q, text, unmount } = await mount(state([device("local", { kit: kit(set) })]));
  expect(q('[data-agent-status="local:claude-code:sessions"]')?.textContent).toContain("2 sessions");
  // A healthy agent with no session says nothing.
  expect(q('[data-agent-row="local:codex:on"] [data-agent-status]')).toBeNull();
  expect(text()).not.toContain("Ready");
  // A count is all a row says about sessions: no list to open and no pane to go to.
  expect(q('[data-agent-row="local:claude-code:on"] button:not([role="switch"]):not([data-agent-permission-select])')).toBeNull();
  expect(text()).not.toContain("connected");
  // Partial: the chip only, on or off, never a count.
  expect(q('[data-agent-partial="grok"]')).not.toBeNull();
  expect(q('[data-agent-row="local:grok:off"] [data-agent-status]')).toBeNull();
  // A Full agent never wears the chip.
  expect(q('[data-agent-partial="codex"]')).toBeNull();
  await unmount();
});

it("shows any broken piece as one plain line on its own row with Reinstall, naming the piece only in its hover and description (B13, B20)", async () => {
  const set = [
    agent("claude-code", { hook: { state: "removed", why: null, location: "/Users/me/.claude/settings.json" }, skill: piece("failed", "not_changed") }),
    agent("codex"),
    agent("opencode", { skill: piece("outdated") }),
  ];
  const { q, all, click, events, unmount } = await mount(state([device("local", { kit: kit(set) })]));
  const line = q('[data-agent-problem="local:claude-code"]')!;
  expect(visible(line)).toBe("✕Disconnected from HideReinstall");
  const detail = line.getAttribute("title");
  expect(detail).toContain("Skill: Failed: Hide could not install or change it.");
  expect(detail).toContain("Hook: Removed · /Users/me/.claude/settings.json");
  const button = q('[data-hook-reinstall="local:claude-code"]')!;
  expect(document.getElementById(button.getAttribute("aria-describedby")!)?.textContent).toBe(detail);
  // A skill or a plugin breaking reads the same.
  expect(visible(q('[data-agent-problem="local:opencode"]')!)).toBe("!Disconnected from HideReinstall");
  expect(all("[data-agent-problem]")).toHaveLength(2);
  // No piece is named anywhere on the page.
  expect(visible(q("[data-agents-machine-list]")!)).not.toMatch(/Hook|Plugin|Extension|Skill:/);
  await click(q('[data-hook-reinstall="local:claude-code"]'));
  expect(sent(events, "kit_reinstall")[0]?.payload).toEqual({ device_id: "local", components: [], agents: ["claude-code"] });
  await unmount();
});

it("shows a copy too old for Hide as the versions, the one command that updates it and its path, with no Reinstall", async () => {
  const update = { installed: "2.1.200", minimum: "2.1.278", path: "/Users/me/.local/bin/claude", command: "claude update" };
  const set = [agent("claude-code", { update, hook: piece("failed", "update_agent"), skill: piece("failed", "update_agent") }), agent("codex")];
  const { q, all, unmount } = await mount(state([device("local", { kit: kit(set) })]));
  const guide = q('[data-agent-update="claude-code"]')!;
  expect(guide.textContent).toContain("Update 2.1.200 to 2.1.278 or later to connect it to Hide");
  expect(guide.textContent).toContain("claude update");
  expect(guide.textContent).toContain("/Users/me/.local/bin/claude");
  expect(all("[data-agent-problem]")).toHaveLength(0);
  expect(q('[data-hook-reinstall="local:claude-code"]')).toBeNull();
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
  const failed = { id: "cli", state: "failed", why: { cause: "not_changed" }, location: null } as Kit["components"][number];
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
  const { q, click, events, unmount } = await mount(state([device("local", { kit: kit(SEVEN, { unavailable: "standalone" }) })]));
  expect(q("[data-agents-unavailable]")?.textContent).toBe("This Hide is not running from the installed app, so it installs nothing on this device. Open the Hide app to install.");
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

it("lists an agent that is on only by default, with no program, as not installed (B9)", async () => {
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
  expect(again.q('[data-agent-row="local:claude-code:on"] [data-agent-status]')).toBeNull();
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
        skill: { state: "absent", why: { cause: "program_missing", value: id }, location: null },
        hook: { state: "absent", why: { cause: "program_missing", value: id }, location: null },
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
  expect(groups.map((group) => group.getAttribute("data-agent-feature-group"))).toEqual(["basics", "sessions", "collaboration"]);
  expect(groups.map((group) => [...group.querySelectorAll("[data-agent-feature]")].map((line) => line.getAttribute("data-agent-feature")?.split(":")[0]))).toEqual([
    ["skill", "guidance", "start"],
    ["titles", "sleep", "fork"],
    ["letters", "bell", "memory"],
  ]);
  expect(popover.querySelector('[data-agent-feature="letters:no"]')?.textContent).toContain("–Not supported: Letters and Observer warnings");
  expect(popover.querySelector('[data-agent-feature="skill:yes"]')?.textContent).toContain("✓Supported: The hide skill");
  // No row says an agent's status is judged from the screen.
  expect(popover.textContent).not.toContain("from the screen");
  await unmount();
});

it("lists the three skills Hide installs, each with its mark, what it is for and the help it points at (PRD hide-plan B20)", async () => {
  const { all, q, unmount } = await mount(state([device("local")]));
  expect(all("[data-agents-skill]").map((row) => row.getAttribute("data-agents-skill"))).toEqual(["hide", "hide-mission", "hide-plan"]);
  expect(q('[data-agents-skill="hide-plan"]')?.textContent).toBe("hide-planPut up a plan, get it reviewed and keep it current · hide plan help");
  expect(q('[data-agents-skill="hide-mission"] svg')?.getAttribute("class")).toContain("lucide-waypoints");
  expect(q('[data-agents-skill="hide-plan"] svg')?.getAttribute("class")).toContain("lucide-list-checks");
  await unmount();
});

it("puts each installed agent's permission in its row's 권한 column, full access by default, with the command each choice runs in the open menu", async () => {
  const set = [agent("claude-code"), agent("codex"), agent("opencode"), agent("pi")];
  const { q, all, text, unmount } = await mount(state([device("local", { kit: kit(set) })]));
  // One line per agent: no command on the row until the menu opens.
  expect(all("[data-agent-permission-command]")).toHaveLength(0);
  expect(q('[data-agent-row="local:claude-code:on"]')?.querySelector("[data-agent-permission-select]")?.textContent).toBe("Allow everything");
  // The flag each CLI's --help names (read 2026-11-01), after its program, under its choice.
  const menu = await openMenu(q('[data-agent-permission-select="local:claude-code"]')!);
  expect(menu).toEqual(["Allow everythingclaude --dangerously-skip-permissions", "Ask every timeclaude", "Custom…"]);
  await act(async () => { document.activeElement?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
  expect((await openMenu(q('[data-agent-permission-select="local:codex"]')!))[0]).toBe("Allow everythingcodex -c approval_policy=never -c sandbox_mode=danger-full-access");
  await act(async () => { document.activeElement?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
  expect((await openMenu(q('[data-agent-permission-select="local:opencode"]')!))[0]).toBe("Allow everythingopencode --auto");
  await act(async () => { document.activeElement?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
  // Pi asks nothing, so its column says so and offers no choice.
  expect(q('[data-agent-permission="local:pi:not-asked"]')?.textContent).toBe("Never asks");
  expect(q('[data-agent-permission-select="local:pi"]')).toBeNull();
  // The column heads, and the box under the card that says what the choice covers.
  expect(q("[data-agents-columns]")?.textContent).toBe("PermissionsOn");
  expect(q("[data-agents-permission-scope]")?.textContent).toBe(
    "Permissions✓Applies to every agent Hide starts, including agents another agent starts✕claude or codex typed into a terminal follows the CLI's own settings",
  );
  expect(text()).not.toContain("⌘N");
  await unmount();
});

it("reads each machine's own permission, adds nothing for Ask every time, and saves typed options once on Enter", async () => {
  const next = state([device("local", { kit: kit([agent("claude-code"), agent("codex")]) }), device("studio", { kit: kit([agent("claude-code")]) })]);
  next.rest.ui_state = {
    agent_permissions: { "claude-code": { mode: "ask" }, codex: { mode: "custom", text: "--full-auto", options: ["--full-auto"] } },
    device_agent_permissions: {},
  } as never;
  const { q, events, unmount } = await mount(next);
  expect(q('[data-agent-permission="local:claude-code:ask"]')?.textContent).toBe("Ask every time");
  expect((await openMenu(q('[data-agent-permission-select="local:claude-code"]')!))[1]).toBe("Ask every timeclaude");
  await act(async () => { document.activeElement?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
  const input = q('[data-agent-permission-input="local:codex"]') as HTMLInputElement;
  expect(input.value).toBe("--full-auto");
  expect(input.parentElement?.textContent).toContain("codex");
  const type = async (value: string) => {
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
  };
  await type("--full-auto --search");
  await act(async () => { input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
  await act(async () => { input.dispatchEvent(new FocusEvent("focusout", { bubbles: true })); });
  expect(sent(events, "agent_permission_set").map((event) => event.payload)).toEqual([
    { device_id: "local", agent: "codex", mode: "custom", text: "--full-auto --search" },
  ]);
  await unmount();
});

it("says why a hook waits in plain words, naming the piece only in its hover (PRD opencode-plugin B14)", async () => {
  const waiting = { state: "absent" as const, why: { cause: "not_created_yet", value: "~/.config/opencode" }, location: null };
  const { q, unmount } = await mount(state([device("local", { kit: kit([agent("opencode", { hook: waiting })]) })]));
  const line = q('[data-agent-hook-wait="local:opencode"]')!;
  expect(line.textContent).toBe("Hide puts it in once the agent makes this folder: ~/.config/opencode");
  expect(line.parentElement?.getAttribute("title")).toContain("Plugin");
  expect(q("[data-agent-problem]")).toBeNull();
  await unmount();
});

it("names each agent file Hide left out on the core's machine with its reason dimmed, and asks for a fresh read only while the core publishes them (engine-bundle-3 B1, B24)", async () => {
  const off = await mount(state([device("local")]));
  expect(off.q("[data-agents-definitions]")).toBeNull();
  expect(sent(off.events, "agent_definitions_read")).toHaveLength(0);
  await off.unmount();

  const on = state([device("local")]);
  (on.rest.status as Record<string, unknown>).agent_definitions = {
    rejected: [{ file: "mypi.toml", reason: "mypi.toml: based_on foo is not a built-in agent" }],
    unreadable: null,
  };
  const shown = await mount(on);
  expect(sent(shown.events, "agent_definitions_read")).toHaveLength(1);
  const row = shown.q('[data-agents-definition-left-out="mypi.toml"]');
  expect(row?.textContent).toContain("mypi.toml");
  expect(row?.textContent).toContain("Not loaded");
  expect(row?.querySelector("p")?.className).toContain("text-muted-foreground");
  expect(row?.querySelector("p")?.textContent).toBe("mypi.toml: based_on foo is not a built-in agent");
  await shown.unmount();

  const clean = state([device("local")]);
  (clean.rest.status as Record<string, unknown>).agent_definitions = { rejected: [], unreadable: null };
  const quiet = await mount(clean);
  expect(quiet.q("[data-agents-definitions]")).toBeNull();
  await quiet.unmount();
});
