import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { emptyScope } from "../test/legacyAgentScope";
import { deviceTerminalCount, installParts, SWITCH_STEPS, switchDrawn, switchFailureKey, switchHeld, switchOffered, switchStep } from "./deviceSwitch";
import type { Device, DeviceSwitch, SnapshotRest } from "./snapshot";

const device = (over: Partial<Device> = {}): Device => ({ agent_scope: emptyScope(), id: "mini", label: "Mac mini", kind: "remote", state: "ready", message: null, ssh_alias: "mini", agent_count: 0, test: null, switchable: true, ...over });

describe("which device offers the switch (B35)", () => {
  it("is the device the core says can be switched, and no other", () => {
    expect(switchOffered(device())).toBe(true);
    // Not connected now: the press itself says whether SSH reaches it.
    expect(switchOffered(device({ state: "unavailable" }))).toBe(true);
    // A WSL distribution of this PC is offered it like an SSH device (D-27).
    expect(switchOffered(device({ ssh_alias: null, wsl_distro: "Ubuntu" }))).toBe(true);
    // A device that never connected, a paired machine's and an older daemon's row carry no such word.
    expect(switchOffered(device({ switchable: false }))).toBe(false);
    expect(switchOffered(device({ switchable: undefined }))).toBe(false);
  });

  it("names linger for a Linux machine reached over SSH only (B36)", () => {
    expect(installParts("linux", false)).toEqual(["core", "autostart", "linger"]);
    expect(installParts("macos", false)).toEqual(["core", "autostart"]);
    expect(installParts("linux", true)).toEqual(["core", "autostart"]);
    expect(installParts(undefined, false)).toEqual(["core", "autostart"]);
  });
});

describe("what a switch reads as (B36, B41)", () => {
  it("has a step for every state the core sends but the failed one", () => {
    const contract = JSON.parse(fs.readFileSync(path.resolve(__dirname, "../../contracts/snapshot-wire-enums.json"), "utf8")) as { device_switch_state: DeviceSwitch["state"][] };
    expect(contract.device_switch_state.filter((state) => state !== "failed" && state !== "held")).toEqual([...SWITCH_STEPS]);
    expect(contract.device_switch_state.map((state) => switchStep({ state }))).toEqual(["checking", "installing", "connecting", null, null]);
    expect(switchStep(null)).toBeNull();
  });

  it("draws a switch that runs or stopped on the device's row, and one that waits for the operator in its question alone (B69)", () => {
    expect([undefined, { state: "connecting" as const }, { state: "failed" as const, reason: "ssh_not_reached" }, { state: "held" as const, by: "Office Mac" }].map((state) => [switchDrawn(device({ switch: state })), switchHeld(device({ switch: state }))])).toEqual([
      [false, false],
      [true, false],
      [true, false],
      [false, true],
    ]);
  });

  it("words the reasons the operator can act on, and any other as the plain failure", () => {
    expect(switchFailureKey("ssh_not_reached")).toBe("devices.switch.failed.notReached");
    expect(switchFailureKey("wsl_not_reached")).toBe("devices.switch.failed.wslNotReached");
    expect(switchFailureKey("ssh_update_there")).toBe("pairing.ssh.updateThere");
    // A refusal from a Hide of this build is no older Hide (operator review 2026-11-23).
    expect(switchFailureKey("ssh_refused_there")).toBe("pairing.ssh.refusedThere");
    expect(switchFailureKey("ssh_hide_not_running")).toBe("pairing.ssh.notRunning");
    expect(switchFailureKey("switch_not_connected")).toBe("devices.switch.failed.notConnected");
    expect(switchFailureKey("ssh_core_not_started")).toBe("devices.switch.failed.notStarted");
    expect(switchFailureKey("ssh_mac_signed_out")).toBe("pairing.ssh.signInMac");
    expect(switchFailureKey("ssh_mac_from_mac_only")).toBe("devices.switch.failed.macOnly");
    expect(switchFailureKey("switch_not_saved")).toBe("devices.switch.failed.notSaved");
    expect(switchFailureKey("a_code_of_a_newer_daemon")).toBe("devices.switch.failed.other");
    expect(switchFailureKey(null)).toBe("devices.switch.failed.other");
  });
});

describe("how many terminals a switch with a folder of its own closes (B69)", () => {
  it("adds the device's projects, the core's and the device's own, and no other device's", () => {
    const workspace = (id: string, deviceId: string, panes: number) => ({ id, device_id: deviceId, removal: { pane_count: panes, running_agent_count: 0 }, checkouts: [] });
    const rest = {
      navigator: { workspaces: [workspace("a", "mini", 2), workspace("b", "local", 5)] },
      status: { remote: [{ target_id: "mini", state: "connected", session: { workspaces: [workspace("c", "mini", 1)] } }] },
    } as unknown as SnapshotRest;
    expect(deviceTerminalCount(rest, "mini")).toBe(3);
    expect(deviceTerminalCount(rest, "studio")).toBe(0);
    expect(deviceTerminalCount(null, "mini")).toBe(0);
  });
});
