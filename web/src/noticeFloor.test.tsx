// @vitest-environment jsdom
import { act, useRef } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useNoticeFloor } from "./noticeFloor";
import { useUiStore } from "./ui";

// The window's corner notices stand above every input box drawn at its foot, the highest of them, and come back down
// when the boxes go (UI_BEHAVIOR.md, Notices).

let mounted: ReturnType<typeof createRoot>[] = [];

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  vi.stubGlobal("innerHeight", 900);
  useUiStore.getState().setNoticeFloor(0);
});

afterEach(async () => {
  for (const root of mounted) await act(async () => root.unmount());
  mounted = [];
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

/** A box whose top stands `top` pixels down the window. */
function Box({ top }: { top: number }) {
  const ref = useRef<HTMLDivElement>(null);
  useNoticeFloor(ref);
  return <div ref={(node) => {
    if (node) node.getBoundingClientRect = () => ({ top, bottom: 900, left: 0, right: 100, width: 100, height: 900 - top, x: 0, y: top, toJSON: () => ({}) });
    ref.current = node;
  }} />;
}

async function draw(top: number) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<Box top={top} />));
  return root;
}

it("stands above the highest box drawn and comes back to the corner when every box goes", async () => {
  const low = await draw(820);
  expect(useUiStore.getState().noticeFloor).toBe(80);
  const high = await draw(780);
  expect(useUiStore.getState().noticeFloor).toBe(120);
  await act(async () => high.unmount());
  expect(useUiStore.getState().noticeFloor).toBe(80);
  await act(async () => low.unmount());
  expect(useUiStore.getState().noticeFloor).toBe(0);
});
