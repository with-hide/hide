// @vitest-environment jsdom
// The empty Home of a desktop app (PRD first-run-onboarding D-16, B27, B28,
// B51, B52) as the onboarding scene draws it: the shell's real Sidebar and
// Overview over the scene's synthetic Mac, with no project and with a project
// but no agent. The scene's actions are the real ones; what a press opens is
// read from the stores it opens.

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { english } from "../i18n/catalogs";
import { useStartPanel } from "../startDraft";
import { useUiStore } from "../ui";
import { OnboardingScene, type OnboardingState } from "./OnboardingScene";
import tokensText from "../../../design/tokens.json?raw";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

let root: Root | null = null;

async function scene(state: OnboardingState) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  // jsdom loads no stylesheet, and the sidebar reads its width tokens from the document: take them from the token source.
  const tokens = (JSON.parse(tokensText) as { tokens: Record<string, { value: number }> }).tokens;
  for (const name of ["--size-sidebar-ideal", "--size-sidebar-min", "--size-sidebar-max"]) document.documentElement.style.setProperty(name, `${tokens[name]?.value}px`);
  const container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  await act(async () => root!.render(<OnboardingScene theme="light" state={state} system="mac" language="en" scale={1} />));
  return container;
}

afterEach(async () => {
  await act(async () => root?.unmount());
  root = null;
  document.body.replaceChildren();
  useStartPanel.setState({ isOpen: false });
  vi.unstubAllGlobals();
});

it("with no project, says what to do with Add project and its chord in the middle of Home, and the button opens Add project (B27)", async () => {
  const container = await scene("home-no-project");
  const home = container.querySelector('[data-main-empty-home="no_project"]')!;
  expect(home.querySelector("p")?.textContent).toBe(english["firstRun.home.noProject"]);
  const button = home.querySelector('[data-main-empty-home-action="new_workspace"]') as HTMLButtonElement;
  expect(button.textContent).toBe(english["firstRun.home.addProject"]);
  expect(button.nextElementSibling?.textContent).toBe("⇧⌘N");
  // The sidebar's only other way to add one is the Add project row under Overview (B51, B52).
  expect(container.querySelectorAll("[data-sidebar-new-workspace]")).toHaveLength(1);
  await act(async () => button.click());
  expect(useUiStore.getState().overlay).toBe("add_project");
  await act(async () => useUiStore.getState().closeOverlay("add_project"));
  // The Projects tab says the same in its place (PRD sidebar-task-lens B42).
  await act(async () => useUiStore.getState().setMainView("projects"));
  expect(container.querySelector('[data-main-view="projects"] [data-main-empty-home="no_project"]')).not.toBeNull();
});

it("with projects and no agent, keeps the backlog rather than an empty Home; the sidebar's 태스크 list offers New agent (PRD sidebar-task-lens B42)", async () => {
  const container = await scene("home-no-agent");
  expect(container.querySelector("[data-main-empty-home]")).toBeNull();
  expect(container.querySelector("[data-overview-backlog]")).not.toBeNull();
});
