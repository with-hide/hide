import { expect, it } from "vitest";
import type { SwitchCondition } from "../pairing";
import { stepState, switchButton, switchFailureKey } from "./switchWays";

// The operator's table (2026-11-24): which machine can be switched by a press.
it("lets a press switch a machine that is warned about or asked to linger, and names the ones it cannot", () => {
  const button = (condition: SwitchCondition) => switchButton(condition);
  // A Mac that does not log in by itself is switched with a warning, not refused.
  expect(button({ kind: "auto_login_on" })).toBe("on");
  expect(button({ kind: "auto_login_off", how: true })).toBe("on");
  expect(button({ kind: "auto_login_off", how: false })).toBe("on");
  // Linux: lingering, or Hide may turn it on at the press.
  expect(button({ kind: "linger_on" })).toBe("on");
  expect(button({ kind: "linger_off" })).toBe("on");
  // Off, with the one thing to change beside it.
  expect(button({ kind: "signed_out" })).toBe("off");
  expect(button({ kind: "linger_admin", command: "sudo loginctl enable-linger me" })).toBe("off");
  // Never drawn: nothing set there changes it.
  expect(button({ kind: "no_systemd" })).toBe("none");
  expect(button({ kind: "windows" })).toBe("none");
  expect(button({ kind: "unknown" })).toBe("none");
});

it("checks the steps behind the one under way and dims the ones ahead", () => {
  expect(["checking", "starting", "pairing", "connecting", "projects"].map((step) => stepState(step as never, "pairing"))).toEqual(["done", "done", "on", "later", "later"]);
});

it("words a failed press by the step that stopped", () => {
  expect(switchFailureKey("ssh_core_not_started")).toBe("devices.ways.failed.start");
  expect(switchFailureKey("switch_not_connected")).toBe("devices.ways.failed.connect");
  expect(switchFailureKey("ssh_mac_from_mac_only")).toBe("devices.ways.failed.macOnly");
  // A refusal the Add dialog already words keeps those words.
  expect(switchFailureKey("ssh_update_there")).toBe("pairing.ssh.updateThere");
  expect(switchFailureKey("anything_else")).toBe("devices.ways.failed.other");
});
