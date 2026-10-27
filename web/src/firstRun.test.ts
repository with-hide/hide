import { describe, expect, it } from "vitest";
import { accountHome, agentsToUpdate, failedAgents, underHome, firstRunStep, firstStartAgent, notificationState, notSignedIn, tileStatus, whatChanges, wizardSteps } from "./firstRun";
import type { AiProvider, Kit, KitAgent, SnapshotRest } from "./snapshot";

const agent = (id: string, overrides: Partial<KitAgent> = {}): KitAgent => ({
  id,
  label: id,
  availability: "available",
  enabled: false,
  chosen: false,
  skill: { state: "off", why: null, location: `/home/me/.${id}/skills/hide` },
  hook: null,
  doc_url: "https://example.test",
  ...overrides,
});

const provider = (agentId: string, state: string): AiProvider => ({ id: agentId, label: agentId, agent: agentId, state, headline: "", message: null, installed: true, selectable: state === "ready", retry_at_ms: null, model: "", models: [], models_fixed: false, cli_default: true, models_unavailable_reason: null });

const rest = (first_run: unknown) => ({ ui_state: { first_run } }) as unknown as SnapshotRest;

describe("the step on screen", () => {
  it("is the core's, for this flow version, and only the first in a browser tab (D-14)", () => {
    expect(firstRunStep(rest({ version: 1, step: "notifications" }), true)).toBe("notifications");
    expect(firstRunStep(rest({ version: 1, step: "notifications" }), false)).toBeNull();
    expect(firstRunStep(rest({ version: 1, step: "agents" }), false)).toBe("agents");
    expect(firstRunStep(rest({ version: 1, step: "done" }), true)).toBeNull();
    expect(firstRunStep(rest({ version: 2, step: "agents" }), true)).toBeNull();
    expect(firstRunStep(rest(undefined), true)).toBeNull();
  });
});

describe("a tile's line (B8, B9)", () => {
  it("says sign-in only for the agents Hide checks, and never invents it for the others", () => {
    expect(tileStatus(agent("claude-code"), undefined)).toBe("checking");
    expect(tileStatus(agent("claude-code"), [provider("claude-code", "unread")])).toBe("checking");
    expect(tileStatus(agent("codex"), [provider("codex", "ready")])).toBe("ready");
    expect(tileStatus(agent("codex"), [provider("codex", "usage_limited")])).toBe("ready");
    expect(tileStatus(agent("grok"), [provider("grok", "needs_login")])).toBe("not_signed_in");
    expect(tileStatus(agent("opencode"), [provider("opencode", "needs_login")])).toBe("installed");
    expect(tileStatus(agent("antigravity"), [provider("antigravity", "needs_login")])).toBe("not_signed_in");
    // Until the kit has made Hide AI's folder for agy, its provider is dimmed: signed in or not is unknown.
    expect(tileStatus(agent("antigravity"), [provider("antigravity", "unsupported")])).toBe("installed");
    expect(tileStatus(agent("grok", { availability: "not_installed" }), undefined)).toBe("not_installed");
  });

  it("says an agent too old for Hide needs an update, whatever its sign-in, and lists it with what updates it", () => {
    const update = { installed: "0.1.0", minimum: "0.2.0", path: "/home/me/.local/bin/omp", command: "omp update" };
    expect(tileStatus(agent("omp", { update }), undefined)).toBe("update_needed");
    expect(tileStatus(agent("codex", { update }), [provider("codex", "ready")])).toBe("update_needed");
    expect(agentsToUpdate([agent("omp", { update }), agent("codex"), agent("pi", { availability: "not_installed", update })]).map(({ id }) => id)).toEqual(["omp"]);
  });
});

describe("What changes (B11)", () => {
  it("lists each place for the agents on, one row per place naming every agent that shares it, skips a piece the agent lacks, and ends with the hide link", () => {
    const kit = { components: [{ id: "cli", label: "hide", state: "off", why: null, location: "/home/me/.local/bin/hide" }] } as unknown as Kit;
    const shared = { state: "off" as const, why: null, location: "/home/me/.agents/skills/hide/SKILL.md" };
    const agents = [
      agent("claude-code", { hook: { state: "off", why: null, location: "/home/me/.claude/settings.json" }, herdr: { state: "absent", why: { cause: "agent_absent" }, location: null } }),
      agent("codex", { skill: shared, hook: { state: "off", why: null, location: "/home/me/.codex/hooks.json" } }),
      agent("opencode", { label: "OpenCode", skill: shared, hook: { state: "off", why: null, location: "/home/me/.config/opencode/plugins/hide.js" } }),
      agent("pi", { skill: shared }),
    ];
    expect(whatChanges(agents, new Set(["claude-code", "codex", "opencode"]), kit)).toEqual({
      rows: [
        { path: "/home/me/.claude/settings.json", labels: ["claude-code"], part: "hook" },
        { path: "/home/me/.claude-code/skills/hide", labels: ["claude-code"], part: "skill" },
        { path: "/home/me/.codex/hooks.json", labels: ["codex"], part: "hook" },
        { path: "/home/me/.agents/skills/hide/SKILL.md", labels: ["codex", "OpenCode"], part: "skill" },
        { path: "/home/me/.config/opencode/plugins/hide.js", labels: ["OpenCode"], part: "plugin" },
      ],
      cli: "/home/me/.local/bin/hide",
    });
  });

  it("writes the account home as ~ on every system, and a place outside it as it is (D-26)", () => {
    expect(accountHome("/Users/me/hide")).toBe("/Users/me");
    expect(accountHome("C:\\Users\\me\\hide")).toBe("C:\\Users\\me");
    expect(underHome("/Users/me/.claude/settings.json", "/Users/me")).toBe("~/.claude/settings.json");
    expect(underHome("C:\\Users\\me\\.codex\\hooks.json", "C:\\Users\\me")).toBe("~\\.codex\\hooks.json");
    expect(underHome("/Users/alice/.claude", "/Users/al")).toBe("/Users/alice/.claude");
    expect(underHome("/opt/hide", "/Users/me")).toBe("/opt/hide");
    const kit = { components: [{ id: "cli", label: "hide", state: "off", why: null, location: "/Users/me/.local/bin/hide" }] } as unknown as Kit;
    const herdr = agent("claude-code", { skill: { state: "off", why: null, location: "/Users/me/.claude/skills/hide" }, herdr: { state: "off", why: null, location: "/Users/me/.config/herdr/integrations/claude" } });
    expect(whatChanges([herdr], new Set(["claude-code"]), kit, "/Users/me")).toEqual({
      rows: [
        { path: "~/.claude/skills/hide", labels: ["claude-code"], part: "skill" },
        { path: "~/.config/herdr/integrations/claude", labels: ["claude-code"], part: "herdr" },
      ],
      cli: "~/.local/bin/hide",
    });
  });
});

describe("a failed agent (B12)", () => {
  it("is a chosen agent that is on with a failed piece, named with the kit's reason", () => {
    const agents = [agent("claude-code", { enabled: true, hook: { state: "failed", why: { cause: "not_changed" }, location: "/x" } }), agent("codex", { enabled: true, skill: { state: "failed", why: { cause: "not_changed" }, location: "/y" } })];
    expect(failedAgents(agents, ["claude-code"])).toEqual([{ id: "claude-code", label: "claude-code", why: { cause: "not_changed" } }]);
  });

  it("is not an agent whose piece was refused only because its copy is too old: the update installs it", () => {
    const old = agent("omp", { enabled: true, hook: { state: "failed", why: { cause: "update_agent", value: "0.2.0" }, location: "/x" } });
    expect(failedAgents([old], ["omp"])).toEqual([]);
  });
});

describe("the start panel's first agent (D-15)", () => {
  it("is the first signed-in agent that is on, then the first that is on, then none", () => {
    const on = (id: string) => agent(id, { enabled: true });
    expect(firstStartAgent([on("claude-code"), on("codex")], [provider("claude-code", "needs_login"), provider("codex", "ready")])).toBe("codex");
    expect(firstStartAgent([on("opencode"), on("codex")], [provider("codex", "needs_login")])).toBe("opencode");
    expect(firstStartAgent([agent("codex")], [provider("codex", "ready")])).toBeNull();
    // A remote kit has no claim about the local account's sign-in.
    expect(firstStartAgent([on("opencode"), on("codex")], undefined)).toBe("opencode");
  });
});

it("skips Devices only when discovery proved there are no saved profiles", () => {
  const steps = ["agents", "devices", "notifications", "project"];
  expect(wizardSteps(undefined)).toEqual(steps);
  for (const state of ["idle", "loading", "failed"] as const) expect(wizardSteps({ state, error: null, profiles: [], truncated: false })).toEqual(steps);
  expect(wizardSteps({ state: "ready", error: null, profiles: [], truncated: false })).toEqual(["agents", "notifications", "project"]);
});

describe("the start panel's sign-in mark (B23, D-06)", () => {
  it("marks only an agent whose sign-in Hide checks and read as signed out", () => {
    expect(notSignedIn("claude", [provider("claude-code", "needs_login")])).toBe(true);
    expect(notSignedIn("claude", [provider("claude-code", "ready")])).toBe(false);
    expect(notSignedIn("claude", [provider("claude-code", "unread")])).toBe(false);
    expect(notSignedIn("opencode", [provider("opencode", "needs_login")])).toBe(false);
    expect(notSignedIn("terminal", [provider("claude-code", "needs_login")])).toBe(false);
    expect(notSignedIn("codex", undefined)).toBe(false);
  });
});

describe("the notification line (B14-B19, B30)", () => {
  it("follows the switch and what the system says", () => {
    expect(notificationState(false, "granted", "settings")).toBe("off");
    expect(notificationState(true, "not_determined", "settings")).toBe("asking");
    expect(notificationState(true, "granted", "settings")).toBe("on");
    expect(notificationState(true, "denied", "settings")).toBe("blocked");
    expect(notificationState(false, "denied", "wizard")).toBe("blocked");
    expect(notificationState(true, "unknown", "settings")).toBe("unreadable");
    expect(notificationState(true, "unsupported", "wizard")).toBe("on_unchecked");
    expect(notificationState(false, "unsupported", "wizard")).toBe("off");
  });

  it("opens the wizard's step on where macOS already allows Hide, and says nothing until the first read (B16)", () => {
    expect(notificationState(false, "granted", "wizard")).toBe("on");
    expect(notificationState(false, "not_determined", "wizard")).toBe("off");
    expect(notificationState(true, null, "settings")).toBe("reading");
    expect(notificationState(false, null, "wizard")).toBe("off");
  });

  it("says only that nothing can be shown where no notification service answers, whatever the switch says (B47, B48)", () => {
    expect(notificationState(false, "no_service", "wizard")).toBe("no_service");
    expect(notificationState(true, "no_service", "settings")).toBe("no_service");
  });
});
