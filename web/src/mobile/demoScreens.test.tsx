// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { openDetail, stop, start } from "./connection";
import { PhoneLanguageBoundary } from "./language";
import { SettingsScreen } from "./SettingsScreen";
import { PRIVACY_POLICY_URL } from "./SettingsParts";
import { setTab, openMission, usePhone } from "./store";
import { blockNetwork, type NetworkGuard } from "./demoNetworkGuard";

// What a person sees of the sample host: the marks that say it is a sample, on the screens a real host draws, with the sample data in them.

let network: NetworkGuard;
let root: Root;
let host: HTMLElement;
const text = () => document.body.textContent ?? "";
const settle = (ms = 400) => act(async () => { await vi.advanceTimersByTimeAsync(ms); });

beforeAll(async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.useFakeTimers();
  network = blockNetwork((name, value) => vi.stubGlobal(name, value), () => vi.unstubAllGlobals());
  window.history.replaceState(null, "", "/p/5/?demo=1");
  start();
  host = document.body.appendChild(document.createElement("div"));
  root = createRoot(host);
  await act(async () => root.render(<><PhoneLanguageBoundary /><App /></>));
  await settle();
});

afterAll(async () => {
  await act(async () => root.unmount());
  stop();
  host.remove();
  vi.useRealTimers();
  network.release();
  localStorage.clear();
});

describe("the sample host's screens", () => {
  it("say on every screen that the data is a sample, with a way out", () => {
    expect(usePhone.getState().demo).toBe(true);
    const bar = document.querySelector("[data-phone-demo-bar]");
    expect(bar?.textContent).toContain("Demo · sample data");
    expect(bar?.querySelector("[data-phone-demo-exit]")?.textContent).toBe("Leave demo");
    // The host's name carries the pill that marks it as a sample.
    const name = [...document.querySelectorAll("button")].find((button) => button.textContent?.includes("Demo Mac"));
    expect(name?.querySelector("[data-phone-demo-sample]")?.textContent).toBe("Sample");
  });

  it("open no connection of any kind while they are drawn", () => {
    expect(network.calls).toEqual([]);
  });

  it("list the agents by group, with a parent's children under it", () => {
    expect(document.querySelector('[data-phone-group="needs_you"]')).not.toBeNull();
    expect(document.querySelector('[data-phone-group="working"]')).not.toBeNull();
    expect(document.querySelector('[data-phone-group="done"]')).not.toBeNull();
    expect(text()).toContain("Address review comments");
    expect(text()).toContain("Fix the Windows path test");
  });

  it("link a step's worker to its mission and its pull request, and a chip opens that page", async () => {
    await act(async () => openDetail({ device_id: "local", pane_id: "a3" }));
    await settle(50);
    const mission = document.querySelector<HTMLElement>('[data-phone-detail-mission="mission-release"]')!;
    expect(mission.textContent).toContain("Prepare release 1.2.0");
    expect(mission.textContent).toContain("Rehearse the migration");
    expect(document.querySelector("[data-phone-detail-pr]")?.textContent).toContain("2091");
    await act(async () => mission.click());
    expect(usePhone.getState().detail).toBeNull();
    expect(usePhone.getState().tab).toBe("missions");
    expect(usePhone.getState().missionOpen).toBe("mission-release");
    await act(async () => setTab("agents"));
  });

  it("open the host sheet on the sample host, which leads to the real connect flow and back", async () => {
    const trigger = [...document.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent?.includes("Demo Mac"))!;
    await act(async () => trigger.click());
    await settle(50);
    const panel = document.querySelector("[data-phone-demo-panel]");
    expect(panel?.textContent).toContain("Everything here is sample data");
    expect(panel?.querySelector("[data-phone-demo-connect]")?.textContent).toBe("Connect a real computer");
    // No saved host is listed beside it: the sample is the only host the sheet knows.
    expect(document.querySelector("[data-phone-host-revoked]")).toBeNull();
  });

  it("fill the missions and pull request tabs", async () => {
    await act(async () => setTab("missions"));
    expect(text()).toContain("Prepare release 1.2.0");
    await act(async () => openMission("mission-release"));
    expect(text()).toContain("The signing certificate expired");
    await act(async () => setTab("prs"));
    expect(text()).toContain("fix: keep Windows paths on the wire");
    expect(text()).toContain("feat: show children on the phone list");
  });
});

describe("the Settings tab", () => {
  it("links to the privacy policy, opened outside the app", async () => {
    await act(async () => root.render(<><PhoneLanguageBoundary /><SettingsScreen /></>));
    const link = document.querySelector<HTMLAnchorElement>("[data-phone-setting=privacy]")!;
    expect(link.textContent).toBe("Privacy policy");
    expect(link.getAttribute("href")).toBe("https://withhide.dev/privacy");
    expect(PRIVACY_POLICY_URL).toBe(link.href);
    expect(link.target).toBe("_blank");
    expect(link.rel).toContain("noopener");
  });

  it("shows the sample computer with a way out that only leaves the demo, and no notification control", async () => {
    await act(async () => { usePhone.setState({ demo: true }); root.render(<><PhoneLanguageBoundary /><SettingsScreen /></>); });
    expect(document.querySelector("[data-phone-demo-panel]")).not.toBeNull();
    expect(document.querySelector("[data-phone-demo-sample]")).not.toBeNull();
    expect(document.querySelector("[data-phone-demo-exit], [data-phone-demo-connect]")).not.toBeNull();
    await act(async () => usePhone.setState({ demo: false }));
  });
});
