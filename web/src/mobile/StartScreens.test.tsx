// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { TFunction } from "i18next";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { FirstScreen, StartSettings } from "./StartScreens";

let root: Root;
let host: HTMLElement;
const t = ((key: string) => key) as unknown as TFunction;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  host = document.body.appendChild(document.createElement("div"));
  root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });

it("the first screen has the three steps and routes its scan, demo and settings buttons", async () => {
  const calls: string[] = [];
  await act(async () => root.render(<FirstScreen t={t} onScan={() => calls.push("scan")} onDemo={() => calls.push("demo")} onSettings={() => calls.push("settings")} />));
  expect(host.querySelectorAll("ol > li")).toHaveLength(3);
  for (const selector of ["[data-phone-scan]", "[data-phone-demo-entry]", "[data-phone-first-settings]"]) host.querySelector<HTMLElement>(selector)!.click();
  expect(calls).toEqual(["scan", "demo", "settings"]);
});

it("the start settings shows no computer, the display and about rows, and a way back", async () => {
  let back = 0;
  await act(async () => root.render(<StartSettings t={t} onBack={() => { back += 1; }} />));
  expect(host.querySelector('[data-phone-settings-section="computers"]')?.textContent).toContain("mobile.first.noComputers");
  expect(host.querySelector('[data-phone-setting="language"]')).not.toBeNull();
  expect(host.querySelector('[data-phone-setting="privacy"]')).not.toBeNull();
  host.querySelector<HTMLElement>("[data-phone-settings-back]")!.click();
  expect(back).toBe(1);
});
