import { agentPlaces } from "../test/legacyNavigation";
import { describe, expect, it } from "vitest";

import type { Checkout, Device, RemoteStatus, Workspace } from "./snapshot";

function checkout(id: string, branch: string | null, label: string, panes: string[]): Checkout {
  return { id, label, branch, tabs: [{ id: `${id}:t`, label: "1", panes: panes.map((pane) => ({ id: pane })) }] } as unknown as Checkout;
}

function project(label: string, isGit: boolean, checkouts: Checkout[]): Workspace {
  return { id: label, label, is_git: isGit, checkouts } as unknown as Workspace;
}

const local = [
  project("hide", true, [checkout("main", "main", "hide", ["p1"]), checkout("wt", "feat/sidebar", "sidebar", ["p2"])]),
  project("notes", false, [checkout("notes", null, "notes", ["p3"])]),
];
const devices = [{ id: "studio", label: "Studio Mac", kind: "remote" }] as Device[];
const remote = (state: string) =>
  [{ target_id: "studio", state, session: { workspaces: [project("app", true, [checkout("r", "main", "app", ["remote:studio:pane:1"])])] } }] as unknown as RemoteStatus[];

describe("the Agents list's context line (sidebar-readability B5)", () => {
  it("names the project and the checkout's branch, and a plain folder by its project alone", () => {
    const place = agentPlaces(local, [], devices);
    expect(place(null, "p1")).toBe("hide › main");
    expect(place(null, "p2")).toBe("hide › feat/sidebar");
    expect(place(null, "p3")).toBe("notes");
  });

  it("places a device's row in that device's workspaces, and makes none up for a pane no checkout holds", () => {
    const place = agentPlaces(local, remote("connected"), devices);
    expect(place("Studio Mac", "remote:studio:pane:1")).toBe("app › main");
    expect(place("Studio Mac", "p1")).toBeNull();
    expect(place(null, "gone")).toBeNull();
    expect(agentPlaces(local, remote("stale"), devices)("Studio Mac", "remote:studio:pane:1")).toBeNull();
  });
});
