import { expect, it } from "vitest";
import { holdOrigin, originOf, releaseOrigin } from "./backlogOrigin";
import type { BacklogOrigin, SnapshotRest } from "./snapshot";

const a = { id: "project-a", device_id: "device-a" };
const b = { id: "project-b", device_id: "device-b" };

it("closes an item's origin only when the last screen showing it leaves", () => {
  // The panel holds it, then the Start dialog opens over it.
  holdOrigin(a, "github:acme/repo#7");
  holdOrigin(a, "github:acme/repo#7");
  holdOrigin(a, "local:/notes#1");
  expect(releaseOrigin(a, "github:acme/repo#7")).toBe(false);
  expect(releaseOrigin(a, "local:/notes#1")).toBe(true);
  expect(releaseOrigin(a, "github:acme/repo#7")).toBe(true);
  expect(() => releaseOrigin(a, "github:acme/repo#7")).toThrow();
});

it("reads each item's own origin, so another window's item does not replace it", () => {
  const origin = (task_key: string) => ({ workspace_id: a.id, device_id: a.device_id, task_key, phase: "ready" }) as BacklogOrigin;
  const rest = { issue_work: { origins: [origin("github:acme/repo#7"), origin("local:/notes#1")] } } as SnapshotRest;
  expect(originOf(rest, a, "local:/notes#1")?.task_key).toBe("local:/notes#1");
  expect(originOf(rest, a, "github:acme/repo#8")).toBeNull();
  expect(originOf(rest, b, "local:/notes#1")).toBeNull();
  expect(originOf(rest, { ...a, device_id: b.device_id }, "local:/notes#1")).toBeNull();
  expect(originOf(rest, null, "local:/notes#1")).toBeNull();
});

it("closes same-path memo owners separately while one still has two screens", () => {
  const key = "local:/repo#1";
  holdOrigin(a, key);
  holdOrigin(a, key);
  holdOrigin(b, key);
  expect(releaseOrigin(b, key)).toBe(true);
  expect(releaseOrigin(a, key)).toBe(false);
  expect(releaseOrigin(a, key)).toBe(true);
});
