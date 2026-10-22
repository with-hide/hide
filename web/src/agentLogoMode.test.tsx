import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, expect, it } from "vitest";
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
