// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useShellStore } from "../store";
import { useFileBytesFrom } from "./useFileBytes";

const reads = vi.hoisted(() => ({ paths: [] as string[], answers: [] as ((bytes: Uint8Array | Error) => void)[] }));
vi.mock("../fileBytes", () => ({
  requestFileBytes: (path: string) => {
    reads.paths.push(path);
    return new Promise<Uint8Array>((resolve, reject) => reads.answers.push((answer) => (answer instanceof Error ? reject(answer) : resolve(answer))));
  },
}));

function Shown({ path }: { path: string }) {
  const state = useFileBytesFrom(path, null);
  return <span data-state={state.status}>{state.status === "ready" ? new TextDecoder().decode(state.bytes) : ""}</span>;
}

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
let saved: ReturnType<typeof useShellStore.getState>;
const shown = () => container.querySelector("span")!;
const answer = async (text: string | Error) => act(async () => reads.answers.shift()!(text instanceof Error ? text : new TextEncoder().encode(text)));
const changed = async (path: string) =>
  act(async () => {
    useShellStore.getState().applyFrame({ type: "file_changed", payload: { path, device_id: "mac" } });
  });

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  saved = useShellStore.getState();
  reads.paths = [];
  reads.answers = [];
  useShellStore.setState({ rest: { navigator: { focused_device_id: "mac" } } as never, fileChanges: {} });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  useShellStore.setState(saved, true);
  vi.unstubAllGlobals();
});

describe("a shown file that changes on disk (file_changed)", () => {
  it("is read again, keeping its last bytes on screen until the new ones arrive", async () => {
    await act(async () => root.render(<Shown path="/r/shot.svg" />));
    await answer("one");
    expect(shown().textContent).toBe("one");

    await act(async () => {
      useShellStore.getState().applyFrame({ type: "file_changed", payload: { path: "/r/other.svg", device_id: "mac" } });
    });
    expect(reads.paths).toEqual(["/r/shot.svg"]);

    await act(async () => {
      useShellStore.getState().applyFrame({ type: "file_changed", payload: { path: "/r/shot.svg", device_id: "mac" } });
    });
    expect(reads.paths).toEqual(["/r/shot.svg", "/r/shot.svg"]);
    expect(shown().dataset.state).toBe("ready");
    expect(shown().textContent).toBe("one");
    await answer("two");
    expect(shown().textContent).toBe("two");

    await changed("/r/shot.svg");
    await answer(new Error("not_found"));
    expect(shown().dataset.state).toBe("ready");
    expect(shown().textContent).toBe("two");
  });

  it("starts another file from loading", async () => {
    await act(async () => root.render(<Shown path="/r/a.svg" />));
    await answer("a");
    await act(async () => root.render(<Shown path="/r/b.svg" />));
    expect(shown().dataset.state).toBe("loading");
  });
});
