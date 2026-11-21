// @vitest-environment jsdom
// The phone's start sheet past hided's cap on places (docs/UI_BEHAVIOR.md, The phone app): the list ends with one
// line that counts the places left out, which cannot be picked, and an older Mac's frame without the count draws none.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { StartSheet } from "./StartSheet";
import type { ServerFrame } from "./protocol";
import { applyFrame, patch, usePhone } from "./store";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
const initial = usePhone.getState();
afterEach(() => {
  usePhone.setState(initial, true);
  vi.unstubAllGlobals();
});

const catalog = (omitted?: number): ServerFrame => ({
  type: "start_catalog",
  targets: [
    { id: "c1", device_id: "local", place: "hide · main", device_label: null, connected: true },
    { id: "c2", device_id: "local", place: "hide · prd/a", device_label: null, connected: true },
  ],
  core_platform: "macos aarch64",
  kinds: [{ id: "claude", models: [] }, { id: "codex", models: [] }],
  remembered: { kind: null, models: {} },
  ...(omitted === undefined ? {} : { omitted }),
});

async function sheetOptions(frame: ServerFrame): Promise<HTMLOptionElement[]> {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  applyFrame(frame);
  patch({ connected: true, startSheet: { ...usePhone.getState().startSheet, open: true } });
  const container = document.createElement("div");
  const root = createRoot(container);
  await act(async () => root.render(<StartSheet />));
  // The sheet draws into the document, not into the container.
  const options = [...document.body.querySelectorAll<HTMLOptionElement>('[data-phone-start-target="true"] option')];
  await act(async () => root.unmount());
  return options;
}

it("ends the place list with one line counting the places the Mac left out", async () => {
  const options = await sheetOptions(catalog(3));
  expect(options.map((option) => option.textContent)).toEqual(["This Mac · hide · main", "This Mac · hide · prd/a", "3 more"]);
  const last = options.at(-1)!;
  expect(last.disabled).toBe(true);
  expect(last.getAttribute("data-phone-start-targets-omitted")).toBe("3");
});

it("draws no such line for a whole list or an older Mac's frame", async () => {
  for (const frame of [catalog(0), catalog()]) {
    const options = await sheetOptions(frame);
    expect(options.map((option) => option.textContent)).toEqual(["This Mac · hide · main", "This Mac · hide · prd/a"]);
    usePhone.setState(initial, true);
  }
});
