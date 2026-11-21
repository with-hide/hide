import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, expect, it } from "vitest";
import { profileInitial } from "./AgentMark";
import { AgentRowMark } from "./AgentRowMark";
import { agentLogosShown } from "./agentLogoMode";
import type { SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

// PRD agent-logo-view-mode D-03, D-05, D-09: the one rule that decides whether a row draws its logo.

const withMode = (mode?: "shown" | "hidden") => useShellStore.setState({ rest: { ui_state: mode ? { agent_logo_mode: mode } : {} } as unknown as SnapshotRest });
const mark = (hideAi: "sidebar" | null = null) => renderToStaticMarkup(<AgentRowMark kind="claude" hideAi={hideAi} />);

afterEach(() => {
  useShellStore.setState({ rest: null });
  useUiStore.getState().setHint(null);
});

it.each([
  [undefined, null, false],
  ["hidden", null, false],
  ["hidden", "devices", false],
  ["hidden", "agents", true],
  ["shown", null, true],
  ["shown", "agents", true],
] as const)("mode %s with hold %s shows logos: %s", (mode, hint, shown) => {
  expect(agentLogosShown(mode, hint)).toBe(shown);
});

it("draws nothing in a row while hidden and a store that never chose reads as hidden", () => {
  expect(mark()).toBe("");
  withMode();
  expect(mark()).toBe("");
  withMode("hidden");
  expect(mark()).toBe("");
});

it("keeps a Hide AI row on its owl alone while hidden", () => {
  withMode("hidden");
  const markup = mark("sidebar");
  expect(markup).toContain("data-hide-mark");
  expect(markup).not.toContain("data-agent-mark");
});

it("draws the provider logo when shown, with the owl as its badge on a row that stands alone", () => {
  withMode("shown");
  expect(mark()).toContain('data-agent-mark="claude"');
  expect(mark()).not.toContain("data-hide-ai-badge");
  expect(mark("sidebar")).toContain('data-agent-mark="claude"');
  expect(mark("sidebar")).toContain("data-hide-ai-badge");
});

it("draws the logo while the Agents hold is revealed and not for the devices hold", () => {
  withMode("hidden");
  useUiStore.getState().setHint("devices");
  expect(mark()).toBe("");
  useUiStore.getState().setHint("agents");
  expect(mark()).toContain('data-agent-mark="claude"');
});

it("draws the neutral mark for a kind with no logo when shown", () => {
  withMode("shown");
  expect(renderToStaticMarkup(<AgentRowMark kind="future-agent" />)).toContain('data-agent-mark="neutral"');
});

// PRD agent-profiles B17: a profile agent's logo wears its profile's first letter, and the badge stands alone while logos are hidden.
it("marks a profile agent with its profile's first letter in both modes and a default agent with none", () => {
  const profiled = (profile: string | null) => renderToStaticMarkup(<AgentRowMark kind="claude" profile={profile} />);
  withMode("hidden");
  expect(profiled("work")).toContain('data-profile-badge="work"');
  expect(profiled("work")).toContain(">W</span>");
  expect(profiled("work")).not.toContain("data-agent-mark");
  expect(profiled(null)).toBe("");
  withMode("shown");
  expect(profiled("work")).toContain('data-agent-mark="claude"');
  expect(profiled("work")).toContain('data-profile-badge="work"');
  expect(profiled("work")).toContain(">W</span>");
  expect(profiled(null)).not.toContain("data-profile-badge");
});

it("keeps a Hide AI agent's owl on the corner a profile badge would take", () => {
  const profiled = (hideAi: "sidebar" | null) => renderToStaticMarkup(<AgentRowMark kind="claude" profile="work" hideAi={hideAi} />);
  withMode("shown");
  expect(profiled("sidebar")).toContain("data-hide-ai-badge");
  expect(profiled("sidebar")).not.toContain("data-profile-badge");
  withMode("hidden");
  expect(profiled("sidebar")).toContain("data-hide-mark");
  expect(profiled("sidebar")).not.toContain("data-profile-badge");
});

it.each([["work", "W"], ["회사", "회"], [" personal", "P"]])("draws profile %s as %s", (profile, letter) => {
  expect(profileInitial(profile)).toBe(letter);
});
