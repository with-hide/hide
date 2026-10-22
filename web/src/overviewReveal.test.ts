// PRD pane-open-in-overview B12: a pane's Show in Overview belongs to the opening it was pressed for;
// leaving the Overview or moving it to another device drops one the map never took.
import { afterEach, expect, it } from "vitest";
import { useUiStore } from "./ui";

const saved = useUiStore.getState();
afterEach(() => useUiStore.setState(saved, true));
const pending = () => useUiStore.setState({ overviewReveal: { deviceId: "local", paneId: "p1", sessionId: "native" }, overview: { ...saved.overview, deviceId: "local" } });

it.each([
  ["closing the Overview", () => useUiStore.getState().setOverviewOpen(false)],
  ["changing screen", () => useUiStore.getState().setScreen({ kind: "workspace" })],
  ["restoring a screen", () => useUiStore.getState().restoreScreen({ kind: "workspace" })],
  ["moving to another device", () => useUiStore.getState().setOverview({ deviceId: "mini" })],
])("drops a reveal the map never took on %s", (_, leave) => {
  pending();
  leave();
  expect(useUiStore.getState().overviewReveal).toBeNull();
});

it("keeps it while the Overview stays on its device", () => {
  pending();
  useUiStore.getState().setOverview({ deviceId: "local", projectId: "alpha" });
  useUiStore.getState().setOverviewOpen(true);
  expect(useUiStore.getState().overviewReveal).toEqual({ deviceId: "local", paneId: "p1", sessionId: "native" });
});
