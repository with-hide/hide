// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { adjacentPane } from "./paneNavigation";

/** DOM geometry is the browser's boundary; the navigation decision stays real. */
function pane(canvas: HTMLElement, id: string, x: number, y: number, width: number, height: number) {
  const leaf = document.createElement("div");
  leaf.dataset.layoutPane = id;
  leaf.innerHTML = `<div data-pane-view="${id}"></div>`;
  leaf.getBoundingClientRect = () => new DOMRect(x, y, width, height);
  canvas.append(leaf);
}
function canvas(tab: string, remote = false) {
  const element = document.createElement("div");
  element.dataset.canvas = tab;
  if (remote) element.dataset.remoteCanvas = "true";
  document.body.append(element);
  return element;
}

afterEach(() => { document.body.replaceChildren(); vi.unstubAllGlobals(); });

describe("directional terminal navigation", () => {
  it.each([false, true])("uses shared edges in the drawn canvas (remote: %s)", (remote) => {
    vi.stubGlobal("CSS", { escape: (id: string) => id });
    const tab = canvas("current", remote);
    pane(tab, "left", 0, 0, 400, 600);
    pane(tab, "top", 400, 0, 400, 200);
    pane(tab, "bottom", 400, 200, 400, 400);
    pane(canvas("other-tab"), "unrelated", 400, 0, 400, 600);
    // The larger shared edge wins; the result is not DOM order or a pane id sort.
    expect(adjacentPane("left", "right")).toBe("bottom");
    expect(adjacentPane("bottom", "up")).toBe("top");
    expect(adjacentPane("top", "down")).toBe("bottom");
    expect(adjacentPane("bottom", "left")).toBe("left");
    expect(adjacentPane("bottom", "right")).toBeNull();
    tab.dataset.zoomed = "true";
    expect(adjacentPane("bottom", "left")).toBeNull();
  });

  it("crosses fractional edges at page zoom, but never jumps a pane-sized gap", () => {
    vi.stubGlobal("CSS", { escape: (id: string) => id });
    const tab = canvas("scaled", true);
    // Observed in Chromium at page zoom: the shared edge differs by 0.000015px.
    pane(tab, "top", 722, 64.80000305175781, 381.20001220703125, 327.20001220703125);
    pane(tab, "bottom", 722, 392, 381.20001220703125, 327.20001220703125);
    pane(tab, "left", 340.8000183105469, 64.80000305175781, 381.20001220703125, 654.4000244140625);
    pane(tab, "distant", 1600, 392, 300, 327);
    expect(adjacentPane("left", "right")).toBe("top");
    expect(adjacentPane("bottom", "up")).toBe("top");
    expect(adjacentPane("top", "down")).toBe("bottom");
    expect(adjacentPane("bottom", "right")).toBeNull();
  });

  it("ignores parked zero-sized panes, a missing owner and a single pane", () => {
    vi.stubGlobal("CSS", { escape: (id: string) => id });
    const tab = canvas("single");
    pane(tab, "only", 0, 0, 400, 600);
    pane(tab, "parked", 400, 0, 0, 0);
    expect(adjacentPane("only", "right")).toBeNull();
    expect(adjacentPane("retired", "left")).toBeNull();
  });
});
