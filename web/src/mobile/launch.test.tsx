// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { App } from "./App";
import { openNoticed } from "./connection";
import { patch, usePhone } from "./store";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
const initial = usePhone.getState();
afterEach(() => { usePhone.setState(initial, true); vi.unstubAllGlobals(); });

it("opens a notification on the conversation without drawing guidance or the agents list while it connects", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("location", { ...window.location, protocol: "capacitor:" });
  const key = { device_id: "computer", pane_id: "agent" };
  patch({ screen: "starting" });
  const container = document.createElement("div");
  const root = createRoot(container);
  await act(async () => root.render(<App />));
  const noLanding = () => {
    expect(container.querySelector("[data-phone-guidance]")).toBeNull();
    expect(container.querySelector("[data-phone-connected]")).toBeNull();
    expect(container.querySelector("[data-phone-tabs]")).toBeNull();
  };
  try {
    noLanding();
    await act(async () => patch({ screen: "app", macName: "Computer", pendingOpen: key }));
    noLanding();
    await act(async () => {
      patch({ groups: [] });
      openNoticed(key);
      patch({ pendingOpen: null });
    });
    noLanding();
    expect(container.querySelector('[data-phone-back="true"]')).not.toBeNull();
  } finally { await act(async () => root.unmount()); }
});

it("shows connection recovery when the notification's computer is offline, and leaves the browser list available", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("location", { ...window.location, protocol: "capacitor:" });
  patch({ screen: "app", pendingOpen: { device_id: "computer", pane_id: "agent" }, unreachable: true });
  const container = document.createElement("div");
  const root = createRoot(container);
  await act(async () => root.render(<App />));
  try {
    expect(container.querySelector('[data-phone-connected="false"]')).not.toBeNull();
    await act(async () => {
      vi.stubGlobal("location", { ...window.location, protocol: "https:" });
      patch({ unreachable: false });
    });
    expect(container.querySelector('[data-phone-connected="false"]')).not.toBeNull();
  } finally { await act(async () => root.unmount()); }
});
