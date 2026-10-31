// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Actions } from "./actions";
import { useDiskWatch } from "./diskWatch";

// PRD project-panel-results B20: the lane's holders reach the core as one
// watch over every project one of them shows, and none once they all go.

function Holder({ actions, projectId }: { actions: Actions; projectId: string | null }) {
  useDiskWatch(actions, projectId);
  return null;
}

describe("the lane's disk watch", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("watches every project a holder shows and nothing once the last goes", async () => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    const watched: string[][] = [];
    const actions = { watchDisk: (ids: readonly string[]) => watched.push([...ids]) } as unknown as Actions;
    const root = createRoot(document.createElement("div"));
    const draw = (ids: (string | null)[]) => act(async () => root.render(<>{ids.map((id, index) => <Holder key={index} actions={actions} projectId={id} />)}</>));
    await draw(["a"]);
    await draw(["a", "b"]);
    await draw(["a", "b", "b"]);
    await draw([null, "b"]);
    await act(async () => root.unmount());
    expect(watched).toEqual([["a"], ["a", "b"], ["b"], []]);
  });
});
