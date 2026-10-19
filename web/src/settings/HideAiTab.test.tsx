// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import type { AiProvider, BackgroundAi } from "../snapshot";
import { useShellStore } from "../store";
import type { DispatchFn } from "../ws";
import { HideAiTab } from "./HideAiTab";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

const provider = (id: string, label: string, over: Partial<AiProvider> = {}): AiProvider => ({
  id,
  label,
  agent: id === "claude" ? "claude-code" : id,
  state: "ready",
  headline: "",
  message: null,
  installed: true,
  selectable: true,
  retry_at_ms: null,
  model: "m",
  models: ["m"],
  models_fixed: false,
  cli_default: false,
  models_unavailable_reason: null,
  ...over,
});

const snapshot = (ai: Partial<BackgroundAi>, kitAgents: unknown[] = [], others: unknown[] = []) => ({
  connection: "live" as const,
  rest: {
    status: { background_ai: { enabled: true, provider: "claude", chosen: true, providers: [provider("claude", "Claude Code"), provider("codex", "Codex")], fallback: [], refusal: null, unavailable_reason: null, ...ai } },
    navigator: { devices: [{ id: "local", machine_name: "Mac mini", kit: { agents: kitAgents } }, ...others] },
    ui_state: { issue_settings: { ai_worktree_name: true, closes_instruction: true } },
  },
});

afterEach(() => {
  document.body.innerHTML = "";
});

async function mount(next: ReturnType<typeof snapshot>) {
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
    root.render(<TooltipProvider><HideAiTab actions={actions} /></TooltipProvider>);
  });
  return { events, text: () => container.textContent ?? "", q: (selector: string) => container.querySelector(selector) as HTMLElement | null, unmount: async () => { await act(async () => root.unmount()); useShellStore.setState(saved, true); } };
}

it("names who is answering and why while another agent stands in (B41), and says nothing once Runs on answers again", async () => {
  const retry = new Date(2026, 9, 6, 15, 10).getTime();
  const standing = await mount(snapshot({ refusal: { provider: "claude", reason: "usage_limited", retry_at_ms: retry, using: "codex" } }));
  expect(standing.q("[data-ai-using='codex']")?.textContent).toMatch(/^.*Using Codex · Claude Code is out of usage until .*3:10.*PM/);
  // The consequence line is for the case nothing answers; here Codex does.
  expect(standing.q("[data-ai-paused]")).toBeNull();
  await standing.unmount();

  const recovered = await mount(snapshot({ refusal: null }));
  expect(recovered.q("[data-ai-using]")).toBeNull();
  await recovered.unmount();
});

it("puts the reason on the Runs on row, and no banner, when the chosen agent cannot answer and nobody is listed (B42)", async () => {
  const { q, text, unmount } = await mount(snapshot({ refusal: { provider: "claude", reason: "needs_login", retry_at_ms: null, using: null }, providers: [provider("claude", "Claude Code", { state: "needs_login", selectable: false }), provider("codex", "Codex")] }));
  expect(q("[data-ai-paused]")?.textContent).toBe("Claude Code is signed out · Hide AI is paused until Claude Code can answer.");
  // The chosen agent stays in the list with its state instead of vanishing (B34).
  expect(q("[data-ai-state='claude:needs_login']")?.textContent).toContain("Not signed in");
  expect(q("[data-ai-using]")).toBeNull();
  expect(text()).not.toContain("Using ");
  await unmount();
});

it("names the core's machine where a sign-in lapsed on it, from a window on another machine, and only there (B15, B1)", async () => {
  const signedOut = { refusal: { provider: "claude", reason: "needs_login" as const, retry_at_ms: null, using: null }, providers: [provider("claude", "Claude Code", { state: "needs_login", selectable: false }), provider("codex", "Codex")] };
  const node = [{ id: "mbp", kind: "remote", label: "MacBook Pro", dials_in: true }];
  window.location.hash = "#token=t&node=mbp";
  const away = await mount(snapshot(signedOut, [], node));
  expect(away.q("[data-ai-state='claude:needs_login']")?.textContent).toContain("Not signed in on Mac mini");
  expect(away.q("[data-ai-paused]")?.textContent).toBe("Claude Code is signed out on Mac mini · Hide AI is paused until Claude Code can answer.");
  await away.unmount();
  const nobody = await mount(snapshot({ provider: null, chosen: false, providers: [provider("codex", "Codex", { state: "needs_login", selectable: false })] }, [{ id: "codex", label: "Codex", availability: "available", enabled: true }], node));
  expect(nobody.q("[data-ai-sign-in]")?.textContent).toBe("Sign in to Codex on Mac mini to use Hide AI");
  await nobody.unmount();
  window.location.hash = "";

  // On the core's own machine nothing names a machine, a node dialing in or not.
  const own = await mount(snapshot(signedOut, [], node));
  expect(own.q("[data-ai-state='claude:needs_login']")?.textContent).not.toContain("Mac mini");
  expect(own.q("[data-ai-paused]")?.textContent).toBe("Claude Code is signed out · Hide AI is paused until Claude Code can answer.");
  await own.unmount();
});

it("asks to sign in to the first agent that is on when nobody is signed in, and not before every row has been read (B47)", async () => {
  const signedOut = [provider("claude", "Claude Code", { state: "needs_login", selectable: false }), provider("codex", "Codex", { state: "needs_login", selectable: false })];
  const agents = [
    { id: "claude-code", label: "Claude Code", availability: "available", enabled: false },
    { id: "codex", label: "Codex", availability: "available", enabled: true },
  ];
  const none = await mount(snapshot({ provider: null, chosen: false, providers: signedOut }, agents));
  expect(none.q("[data-ai-sign-in]")?.textContent).toBe("Sign in to Codex to use Hide AI");
  // Hide's other features keep working: nothing is disabled here but the choice that has no option.
  expect(none.q("[data-ai-provider]")?.hasAttribute("disabled")).toBe(true);
  await none.unmount();

  const reading = await mount(snapshot({ provider: null, chosen: false, providers: signedOut.map((row) => ({ ...row, state: "unread" })) }, agents));
  expect(reading.q("[data-ai-sign-in]")).toBeNull();
  await reading.unmount();

  const nothingOn = await mount(snapshot({ provider: null, chosen: false, providers: signedOut }, [{ id: "claude-code", label: "Claude Code", availability: "available", enabled: false }]));
  expect(nothingOn.q("[data-ai-sign-in]")?.textContent).toBe("Turn on an agent in Agents to use Hide AI");
  await nothingOn.unmount();
});

it("takes the rest out of reach while Use Hide AI is off and sends one event for the switch (B33)", async () => {
  const { events, q, unmount } = await mount(snapshot({ enabled: false }));
  expect(q("[data-hide-ai-body]")?.hasAttribute("inert")).toBe(true);
  expect(q("[data-hide-ai-tab]")?.getAttribute("data-ai-enabled")).toBe("false");
  await act(async () => { q("[data-ai-use]")?.click(); });
  expect(events.filter((event) => event.kind === "ai_settings" && !("observing" in (event.payload as object)))).toEqual([{ schema_version: 2, kind: "ai_settings", payload: { enabled: true } }]);
  await unmount();
});

it("marks a fixed list, keeps the current model when the list failed, and offers CLI default (B36)", async () => {
  const fixed = await mount(snapshot({ providers: [provider("claude", "Claude Code", { models_fixed: true, models: ["auto", "pro"], model: "auto", models_unavailable_reason: "fixed" })] }));
  expect(fixed.q("[data-ai-models-fixed]")).not.toBeNull();
  expect(fixed.q("[data-ai-models-failed]")).toBeNull();
  await fixed.unmount();

  const failed = await mount(snapshot({ providers: [provider("claude", "Claude Code", { models: [], model: "sonnet", models_unavailable_reason: "claude_models_unreadable:exit=2" })] }));
  expect(failed.q("[data-ai-models-failed]")?.textContent).toBe("Couldn't load the model list. Keeping the current model.");
  expect(failed.q("[data-ai-model]")?.textContent).toContain("sonnet");
  await failed.unmount();

  const cli = await mount(snapshot({ providers: [provider("claude", "Claude Code", { models: ["a"], model: "", cli_default: true })] }));
  expect(cli.q("[data-ai-model]")?.textContent).toBe("CLI default");
  await cli.unmount();
});

it("shows no fallback group before an agent is chosen, since there is nothing to fall back from", async () => {
  const { q, unmount } = await mount(snapshot({ provider: null, chosen: false }));
  expect(q('[data-settings-group="hide-ai-fallback"]')).toBeNull();
  await unmount();
});

it("does not call an older peer's unchecked agent signed in: it says the check happens on the first request (B66)", async () => {
  const legacy = provider("legacy-provider", "Legacy provider", { login_checked: false });
  const { q, unmount } = await mount(snapshot({ provider: "legacy-provider", providers: [provider("claude", "Claude Code"), legacy] }));
  expect(q("[data-ai-state='legacy-provider:ready']")?.textContent).toContain("Sign-in is only checked on the first request");
  expect(q("[data-ai-state='legacy-provider:ready']")?.textContent).not.toContain("Signed in");
  await unmount();

  // An agent that does check, and an older daemon that sends no field, still read Signed in.
  const checked = await mount(snapshot({}));
  expect(checked.q("[data-ai-state='claude:ready']")?.textContent).toContain("Signed in");
  await checked.unmount();
});

const weekly = (used: number, resets: number | null = null) => ({ used_percent: used, resets_at_ms: resets });

it("shows the usage limit the core reports, 90 for an older daemon, and No limit as null (B1)", async () => {
  const older = await mount(snapshot({}));
  expect(older.q("[data-ai-usage-limit]")?.getAttribute("data-ai-usage-limit")).toBe("90");
  expect(older.q("[data-ai-usage-limit-summary]")?.textContent).toBe("Up to 90% of each agent's week");
  await older.unmount();

  const chosen = await mount(snapshot({ usage_limit: 80 }));
  expect(chosen.q("[data-ai-usage-limit]")?.getAttribute("data-ai-usage-limit")).toBe("80");
  await chosen.unmount();

  const none = await mount(snapshot({ usage_limit: null }));
  expect(none.q("[data-ai-usage-limit]")?.getAttribute("data-ai-usage-limit")).toBe("none");
  expect(none.q("[data-ai-usage-limit-summary]")?.textContent).toContain("No limit");
  await none.unmount();
});

it("writes a weekly percent after a state on the Runs on and fallback rows, and none for an agent with no figure (B3, B10)", async () => {
  const { q, unmount } = await mount(
    snapshot({
      providers: [provider("claude", "Claude Code", { weekly: weekly(86) }), provider("codex", "Codex", { weekly: null }), provider("pi", "Pi")],
      fallback: [{ provider: "codex", model: "" }, { provider: "pi", model: "" }],
    }),
  );
  expect(q("[data-ai-state='claude:ready']")?.textContent).toContain("Signed in · 86% this week");
  expect(q("[data-ai-fallback='codex'] [data-ai-state]")?.textContent).not.toContain("this week");
  expect(q("[data-ai-fallback='pi'] [data-ai-state]")?.textContent).not.toContain("this week");
  await unmount();
});

it("says Past your limit with the reset time on an agent the core marks past the limit, and names who stands in (B4)", async () => {
  const reset = new Date(2026, 9, 12, 9, 0).getTime();
  const { q, unmount } = await mount(
    snapshot({
      providers: [provider("claude", "Claude Code", { weekly: weekly(93.4, reset), past_limit: true }), provider("codex", "Codex", { weekly: weekly(40) })],
      fallback: [{ provider: "codex", model: "" }],
      refusal: { provider: "claude", reason: "past_limit", retry_at_ms: reset, using: "codex", used_percent: 93.4, limit: 90 },
    }),
  );
  expect(q("[data-ai-state='claude:ready']")?.textContent).toMatch(/Past your limit · 93% this week · resets .*9:00/);
  expect(q("[data-ai-fallback='codex'] [data-ai-state]")?.textContent).toContain("Signed in · 40% this week");
  expect(q("[data-ai-using='codex']")?.textContent).toMatch(/Using Codex · Claude Code is past your 90% limit until .*9:00/);
  expect(q("[data-ai-paused]")).toBeNull();
  await unmount();
});

it("says Hide AI is paused when every agent in the list is past the limit (B5)", async () => {
  const { q, unmount } = await mount(
    snapshot({
      providers: [provider("claude", "Claude Code", { weekly: weekly(93), past_limit: true }), provider("codex", "Codex", { weekly: weekly(96), past_limit: true })],
      fallback: [{ provider: "codex", model: "" }],
      refusal: { provider: "claude", reason: "past_limit", retry_at_ms: null, using: null, used_percent: 93, limit: 90 },
    }),
  );
  expect(q("[data-ai-paused]")?.textContent).toBe("Hide AI is paused · every agent is past your 90% limit this week");
  await unmount();

  // Only Runs on is listed: the sentence names that agent and what it waits for.
  const single = await mount(
    snapshot({
      providers: [provider("claude", "Claude Code", { weekly: weekly(93), past_limit: true })],
      refusal: { provider: "claude", reason: "past_limit", retry_at_ms: null, using: null, used_percent: 93, limit: 90 },
    }),
  );
  expect(single.q("[data-ai-paused]")?.textContent).toBe("Claude Code is past your 90% limit this week · Hide AI is paused until Claude Code can answer.");
  await single.unmount();
});

it("sends the limit as a percent or none, one event for one choice", () => {
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  actions.setAiUsageLimit(95);
  actions.setAiUsageLimit("none");
  expect(events).toEqual([
    { schema_version: 2, kind: "ai_settings", payload: { usage_limit: 95 } },
    { schema_version: 2, kind: "ai_settings", payload: { usage_limit: "none" } },
  ]);
});
