// @vitest-environment jsdom
// A part that cannot draw (docs/UI_BEHAVIOR.md, A part that cannot be drawn):
// its siblings keep drawing, the failure is a diagnostic once until the part
// draws again, and the next change to the page's state draws it again.
import { act, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { RenderBoundary } from "./RenderBoundary";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/**
 * A part that draws unless its input names a row it cannot find, as the
 * Issues board's card did: from the snapshot, or from the page's own state.
 */
function Board() {
  const missing = useShellStore((s) => (s.rest as { missing?: string } | null)?.missing);
  const asked = useUiStore((s) => s.notices[0]?.text);
  if (missing) throw new Error(`Agent scope references a missing row: ${missing}`);
  if (asked) throw new Error(`No row for ${asked}`);
  return <p data-part="board">board</p>;
}

let container: HTMLDivElement;
let root: Root;

const failures = () => useShellStore.getState().diagnostics.filter((line) => line.startsWith("render_failed"));
const shown = (selector: string) => container.querySelector(selector)?.textContent ?? null;
const snapshot = (missing: string | null) => act(() => useShellStore.setState({ rest: (missing ? { missing } : {}) as never }));

beforeEach(() => {
  // React reports every error a boundary catches; this file asserts what the page shows instead.
  vi.spyOn(console, "error").mockImplementation(() => {});
  useShellStore.setState({ rest: null, diagnostics: [], diagnosticsDropped: 0 });
  useUiStore.setState({ notices: [] });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  act(() =>
    root.render(
      <>
        <RenderBoundary region="screen" fallback={<p data-part="failed">failed</p>}>
          <Board />
        </RenderBoundary>
        <p data-part="sidebar">sidebar</p>
      </>,
    ),
  );
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

it("draws its fallback in the part's place and leaves the parts beside it drawn", () => {
  snapshot("wN1:p2");
  expect(shown("[data-part=failed]")).toBe("failed");
  expect(shown("[data-part=board]")).toBeNull();
  expect(shown("[data-part=sidebar]")).toBe("sidebar");
  expect(failures()).toEqual(["render_failed region=screen: Agent scope references a missing row: wN1:p2"]);
});

it("draws the part again on the next snapshot that it can draw", () => {
  snapshot("wN1:p2");
  snapshot(null);
  expect(shown("[data-part=board]")).toBe("board");
  expect(shown("[data-part=failed]")).toBeNull();
});

it("draws the part again on a change of the page's own state too", () => {
  act(() => useUiStore.setState({ notices: [{ id: 1, level: "info", text: "wN1:p2", refreshable: false }] }));
  expect(shown("[data-part=failed]")).toBe("failed");
  act(() => useUiStore.setState({ notices: [] }));
  expect(shown("[data-part=board]")).toBe("board");
});

it("records a failure that repeats on every change once, and again after the part drew in between", () => {
  snapshot("wN1:p2");
  snapshot("wN1:p2");
  snapshot("wN1:p2");
  expect(failures()).toHaveLength(1);
  snapshot("wN1:p3");
  expect(failures()).toHaveLength(2);
  snapshot(null);
  snapshot("wN1:p3");
  expect(failures()).toEqual([
    "render_failed region=screen: Agent scope references a missing row: wN1:p2",
    "render_failed region=screen: Agent scope references a missing row: wN1:p3",
    "render_failed region=screen: Agent scope references a missing row: wN1:p3",
  ]);
});

it("still draws a part again when it failed on its first draw under StrictMode's remount", () => {
  useShellStore.setState({ rest: { missing: "wN1:p2" } as never });
  act(() =>
    root.render(
      <StrictMode>
        <RenderBoundary region="screen" fallback={<p data-part="failed">failed</p>}>
          <Board />
        </RenderBoundary>
      </StrictMode>,
    ),
  );
  expect(shown("[data-part=failed]")).toBe("failed");
  snapshot(null);
  expect(shown("[data-part=board]")).toBe("board");
});

it("settles when two parts fail with a new message each time, rather than drawing each other on each diagnostic", () => {
  let attempts = 0;
  function Restless(): never {
    attempts += 1;
    throw new Error(`attempt ${attempts}`);
  }
  act(() =>
    root.render(
      <>
        <RenderBoundary region="sidebar" fallback={null}>
          <Restless />
        </RenderBoundary>
        <RenderBoundary region="screen" fallback={null}>
          <Restless />
        </RenderBoundary>
      </>,
    ),
  );
  const mounted = failures().length;
  snapshot(null);
  expect(failures()).toHaveLength(mounted + 2);
});
