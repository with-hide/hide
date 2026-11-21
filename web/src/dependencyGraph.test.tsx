// @vitest-environment jsdom
// The dependency graph's placement across snapshots: a snapshot rebuilds the
// graph object on every summary, and the cards must keep the layered layout
// they have rather than fall back to columns and wait for the worker again.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { ElkNode } from "elkjs/lib/elk-api";
import type { LayeredGraph } from "./projectBoard";
import { useShellStore } from "./store";

const layout = vi.fn<(graph: ElkNode) => Promise<ElkNode>>();
vi.mock("./dependencyLayout", async (original) => ({
  ...(await original<typeof import("./dependencyLayout")>()),
  workerElk: () => Promise.resolve({ layout, terminateWorker: () => {} }),
}));
const { DependencyGraphView } = await import("./DependencyGraph");

type Node = { id: string };
const graphOf = (): LayeredGraph<Node> => ({ layers: [[{ id: "A" }], [{ id: "B" }]], edges: [{ from: "A", to: "B" }], unrelated: [] });
const draw = (node: Node) => (
  <div key={node.id} data-dependency-node={node.id}>
    {node.id}
  </div>
);

let root: Root | null = null;
let container: HTMLDivElement;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("CSS", { escape: (value: string) => value });
  // jsdom does not inherit custom properties; the graph reads only its two gap tokens.
  const tokens: Record<string, string> = { "--home-dependency-gap": "96", "--spacing-xl": "24" };
  vi.spyOn(window, "getComputedStyle").mockReturnValue({ getPropertyValue: (name: string) => tokens[name] ?? "" } as CSSStyleDeclaration);
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  layout.mockReset();
  useShellStore.setState({ diagnostics: [] });
});

afterEach(async () => {
  await act(async () => root?.unmount());
  root = null;
  document.body.replaceChildren();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

async function show(graph: LayeredGraph<Node>, flow: "across" | "down" = "across") {
  await act(async () => root!.render(<DependencyGraphView graph={graph} draw={draw} idOf={(node) => node.id} flow={flow} />));
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

it("keeps the layered placement when a snapshot rebuilds the same graph, without asking the worker again", async () => {
  layout.mockImplementation(async (graph) => ({
    ...graph,
    width: 400,
    height: 100,
    children: graph.children!.map((child, index) => ({ ...child, x: index * 200, y: 0 })),
    edges: [{ id: "A>B", sources: ["A"], targets: ["B"], sections: [{ id: "s", startPoint: { x: 0, y: 0 }, endPoint: { x: 200, y: 0 } }] }],
  }));
  await show(graphOf());
  const drawn = () => container.querySelector("[data-dependency-graph]")!.getAttribute("data-dependency-layout");
  expect(drawn()).toBe("layered");
  const card = container.querySelector('[data-dependency-node="B"]');
  await show(graphOf());
  expect(drawn()).toBe("layered");
  expect(container.querySelector('[data-dependency-node="B"]')).toBe(card);
  expect(layout).toHaveBeenCalledOnce();
});

it("lays a placed graph out again when it turns from down to across, and keeps it once turned", async () => {
  layout.mockImplementation(async (graph) => ({
    ...graph,
    width: 400,
    height: 100,
    children: graph.children!.map((child, index) => ({ ...child, x: index * 200, y: 0 })),
    edges: [],
  }));
  await show(graphOf(), "down");
  expect(container.querySelector("[data-dependency-graph]")!.getAttribute("data-dependency-layout")).toBe("layered");
  await show(graphOf(), "across");
  await show(graphOf(), "across");
  expect(layout.mock.calls.map(([graph]) => graph.layoutOptions?.["elk.direction"])).toEqual(["DOWN", "RIGHT"]);
  expect(container.querySelector("[data-dependency-graph]")!.getAttribute("data-dependency-layout")).toBe("layered");
});

it("asks once for a structure the layout refused and says why once, however often the snapshot rebuilds it", async () => {
  layout.mockRejectedValue(new Error("layout failed"));
  await show(graphOf());
  await show(graphOf());
  await show(graphOf());
  expect(container.querySelector("[data-dependency-graph]")!.getAttribute("data-dependency-layout")).toBe("columns");
  expect(layout).toHaveBeenCalledOnce();
  expect(useShellStore.getState().diagnostics.filter((line) => line.includes("dependency graph stays in columns"))).toHaveLength(1);
});
