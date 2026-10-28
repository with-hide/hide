import { emptyScope, legacyRest } from "../test/legacyAgentScope";
import { legacyAgentRow } from "../test/legacyAgentRow";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { deviceScope } from "./agentScope";
import { badgeText, deviceConnected, frontDeviceId, frontTitle as frontTitleIn, homeOf, railShown, readsHere, sidebarBody, tileCounts as drawTileCounts, tileHint as tileHintIn, tileMonogram, tileName as tileNameIn } from "./devices";
import { initializeInterfaceI18n } from "./i18n/instance";
import { numberedDevices } from "./numbering";
import { contextAgents, contextWorkspaces } from "./remote";
import { searchEntries } from "./search";
import { wslPlace } from "./screenMachine";
import type { AgentRow, Device, SnapshotRest, Workspace } from "./snapshot";

const t = initializeInterfaceI18n("en").getFixedT(null, "translation");
const korean = initializeInterfaceI18n("ko").getFixedT(null, "translation");
const tileName = (label: string, connected: boolean, counts: Parameters<typeof tileNameIn>[2]) => tileNameIn(label, connected, counts, t);
const tileHint = (label: string, connected: boolean, counts: Parameters<typeof tileHintIn>[2]) => tileHintIn(label, connected, counts, t);
const frontTitle = (devices: Parameters<typeof frontTitleIn>[0], frontId: string) => frontTitleIn(devices, frontId, t);

function agent(paneId: string, group: string): AgentRow {
  return legacyAgentRow({ id: paneId, pane_id: paneId, identity_label: paneId, agent_kind: "claude", mark: "turn", group, status_code: "idle", changed_at_unix_ms: null, emphasized: false, unread: false, demand: "none", activity: "idle" }) as AgentRow;
}

const LOCAL_AGENTS = [agent("l1", "needs_you"), agent("l2", "needs_you"), agent("l3", "working")];

/** This Mac, a connected `mini` with one Needs You, and an unreachable `build-box` whose stale session still holds one. */
function rest(devices: string[] = ["mini", "build-box"]): SnapshotRest {
  const all = [
    { id: "local", label: "This Mac", kind: "local", state: "local", message: null },
    { id: "mini", label: "mini", kind: "remote", state: "ready", message: null },
    { id: "build-box", label: "build-box", kind: "remote", state: "unavailable", message: null },
  ].filter((device) => device.kind === "local" || devices.includes(device.id));
  return {
    navigator: { focused_device_id: "local", devices: all, workspaces: [{ agent_scope: emptyScope(), id: "home", label: "hide", is_home: true, device_id: "local", checkouts: [] }] },
    ui_state: {
      workspace_registrations: [
        { id: "a", label: "a", path: "/a", device_id: "local", pinned: false },
        { id: "b", label: "b", path: "/b", device_id: "local", pinned: false },
        { id: "home", label: "hide", path: "/Users/example/hide", device_id: "local", pinned: false, home: true },
        { id: "m", label: "m", path: "/m", device_id: "mini", pinned: false },
      ],
    },
    status: {
      remote: [
        { target_id: "mini", state: "connected", session: { agents: [agent("remote:mini:pane:1", "needs_you"), agent("remote:mini:pane:2", "done")], workspaces: [] } },
        { target_id: "build-box", state: "stale", session: { agents: [agent("remote:build-box:pane:1", "needs_you")], workspaces: [] } },
      ],
    },
  } as unknown as SnapshotRest;
}

describe("the device rail's facts (quick device-rail-badges)", () => {
  // This window runs on a Mac, whatever system runs the test (#972 names it by its system).
  beforeEach(() => vi.stubGlobal("navigator", { platform: "MacIntel", userAgent: "" }));
  afterEach(() => vi.unstubAllGlobals());
  it("shows the rail with one device alone, and hides it only when the core says so (B1, B6)", () => {
    expect(railShown(rest([]))).toBe(true);
    expect(railShown(rest())).toBe(true);
    expect(railShown(null)).toBe(true);
    const hidden = rest();
    hidden.ui_state = { ...hidden.ui_state, device_rail_visible: false };
    expect(railShown(hidden)).toBe(false);
  });

  it("counts each device's own Needs You and unseen Done, and leaves Working to the Agents tab", () => {
    const local = [...LOCAL_AGENTS, agent("l4", "done"), agent("l5", "seen")];
    expect(tileCounts(rest(), local, "local")).toEqual({ needs_you: 2, done: 1 });
    // Working and Seen alone mark nothing.
    expect(tileCounts(rest(), [agent("x", "working"), agent("y", "seen")], "local")).toEqual({ needs_you: 0, done: 0 });
    expect(tileCounts(rest(), LOCAL_AGENTS, "mini")).toEqual({ needs_you: 1, done: 1 });
  });

  it("gives a device that is not connected no count, its stale session ignored (B4, B8)", () => {
    expect(deviceConnected(rest(), "build-box")).toBe(false);
    expect(tileCounts(rest(), LOCAL_AGENTS, "build-box")).toEqual({ needs_you: 0, done: 0 });
  });

  it("draws `9+` from ten and the count below it (B4)", () => {
    expect([1, 9, 10, 250].map(badgeText)).toEqual(["1", "9", "9+", "9+"]);
  });

  it("draws a device's monogram from the first letters of its first two words", () => {
    expect(["Mac mini", "Mac Studio", "mini", "build-box", "연구실 빌드 서버", "  spaced  out "].map(tileMonogram)).toEqual(["Mm", "MS", "M", "Bb", "연빌", "So"]);
    // A name macOS hands over decomposed still yields whole syllables.
    expect(tileMonogram("연구실 빌드".normalize("NFD"))).toBe("연빌");
  });

  it("names a tile for assistive technology by device, connection and each count it marks (B4)", () => {
    expect(tileName("mini", true, { needs_you: 1, done: 0 })).toBe("mini, Needs You 1");
    expect(tileName("This Mac", true, { needs_you: 2, done: 1 })).toBe("This Mac, Needs You 2, Unread 1");
    expect(tileName("build-box", false, { needs_you: 0, done: 0 })).toBe("build-box, Not connected");
    expect(tileNameIn("build-box", false, { needs_you: 0, done: 0 }, korean)).toBe("build-box, 연결 안 됨");
    expect(tileName("This Mac", true, { needs_you: 0, done: 0 })).toBe("This Mac");
  });

  it("writes the full counts in a tile's hint, where the pill stops at `9+`", () => {
    expect(tileHint("mini", true, { needs_you: 12, done: 1 })).toBe("mini · Needs You 12 · Unread 1");
    expect(tileHint("build-box", false, { needs_you: 3, done: 0 })).toBe("build-box · Not connected");
    expect(tileHintIn("mini", true, { needs_you: 12, done: 1 }, korean)).toBe("mini · 내 차례 12 · 읽을 것 1");
    expect(tileHint("This Mac", true, { needs_you: 0, done: 0 })).toBe("This Mac");
  });

  it("finds a device's Home once it exists, and none before", () => {
    expect(homeOf(rest(), "local")?.id).toBe("home");
    expect(homeOf(rest(), "mini")).toBeNull();
  });

  it("titles the sidebar's top line by the device in front (B3)", () => {
    const devices = rest().navigator!.devices;
    expect(frontTitle(devices, "local")).toEqual({ name: "This Mac", note: null });
    expect(frontTitle(devices, "mini")).toEqual({ name: "mini", note: "Remote" });
  });
});

// PRD wsl-device B5, B6: this PC's WSL distribution is a device like any other: named where it is, never
// Remote, and choosing its tile scopes the lists to its own items by the same device rules as an SSH device.
describe("this PC's WSL distribution as a device (PRD wsl-device B5, B6)", () => {
  beforeEach(() => vi.stubGlobal("navigator", { platform: "Win32", userAgent: "" }));
  afterEach(() => vi.unstubAllGlobals());
  const WSL = "wsl-ubuntu-1a2b3c4d";
  function windows(focused: string): SnapshotRest {
    const project = (id: string, device: string) => ({ agent_scope: emptyScope(), id, label: id, path: `/${id}`, device_id: device, remote_target_id: device === "local" ? null : device, checkouts: [] });
    return {
      navigator: {
        focused_device_id: focused,
        devices: [
          { id: "local", label: null, kind: "local", state: "local", message: null },
          { id: WSL, label: "WSL Ubuntu", kind: "remote", state: "ready", message: null, ssh_alias: null, wsl_distro: "Ubuntu" },
          { id: "mini", label: "mini", kind: "remote", state: "ready", message: null, ssh_alias: "mini" },
        ],
        workspaces: [project("pc-project", "local")],
      },
      status: {
        remote: [
          { target_id: WSL, state: "connected", session: { agents: [agent("remote:wsl:pane:1", "needs_you")], workspaces: [project("remote:wsl:workspace:app", WSL)] } },
          { target_id: "mini", state: "connected", session: { agents: [agent("remote:mini:pane:1", "done")], workspaces: [project("remote:mini:workspace:m", "mini")] } },
        ],
      },
    } as unknown as SnapshotRest;
  }

  it("names it `WSL Ubuntu · This PC` on the rail, the sidebar title and in search, and never Remote (B5)", () => {
    const devices = windows("local").navigator!.devices;
    expect(frontTitle(devices, WSL)).toEqual({ name: "WSL Ubuntu", note: "This PC" });
    expect(frontTitleIn(devices, WSL, korean)).toEqual({ name: "WSL Ubuntu", note: "이 PC" });
    // An SSH device keeps its word.
    expect(frontTitle(devices, "mini")).toEqual({ name: "mini", note: "Remote" });
    expect(tileMonogram("WSL Ubuntu")).toBe("WU");
    expect(wslPlace(devices, WSL, korean)).toBe("이 PC");
    expect(wslPlace(devices, "mini", korean)).toBeNull();
    expect(tileHintIn(`WSL Ubuntu · ${wslPlace(devices, WSL, korean)}`, true, { needs_you: 1, done: 0 }, korean)).toBe("WSL Ubuntu · 이 PC · 내 차례 1");
    const row = searchEntries(windows("local"), korean).find((entry) => entry.id === `device:${WSL}`);
    expect(row?.subtitle).toBe("이 PC");
  });

  it("scopes the sidebar, tasks, Overview, tile counts and numbers to its own items when its tile is chosen (B6)", () => {
    const local = [agent("l1", "needs_you")];
    const front = legacyRest(windows(WSL), local);
    expect(frontDeviceId(front)).toBe(WSL);
    expect(sidebarBody(front)).toBe("projects");
    expect(contextWorkspaces(front).map((row) => row.id)).toEqual(["remote:wsl:workspace:app"]);
    expect(contextAgents(front, local).map((row) => row.pane_id)).toEqual(["remote:wsl:pane:1"]);
    // Tasks and Overview read the front device's scope, the core's count of that device alone.
    expect(deviceScope(front, frontDeviceId(front))).toBe(front.navigator!.devices!.find((device) => device.id === WSL)!.agent_scope);
    expect(tileCounts(windows(WSL), local, WSL)).toEqual({ needs_you: 1, done: 0 });
    expect(tileCounts(windows(WSL), local, "local")).toEqual({ needs_you: 1, done: 0 });
    // ⌥⌘n numbers the tiles in the rail's order: this PC, then each device as the core lists it.
    expect([...numberedDevices(front.navigator!.devices)]).toEqual([[1, "local"], [2, WSL], [3, "mini"]]);
    // A stopped distribution reads as any device that is not connected: no count, the reconnect body.
    const off = windows(WSL);
    off.status!.remote![0]!.state = "not_connected";
    expect(sidebarBody(legacyRest(off, local))).toBe("disconnected");
    expect(tileCounts(off, local, WSL)).toEqual({ needs_you: 0, done: 0 });
  });
});

function tileCounts(rest: SnapshotRest | null, agents: AgentRow[], id: string) { return drawTileCounts(rest ? legacyRest(rest, agents) : null, agents, id); }

describe("which projects the core keeps as its own (PRD core-host-node-move D-29)", () => {
  it("keeps this machine's projects and a node's that dials the core, never a dialed device's", () => {
    const project = (device: string, remote: string | null) => ({ device_id: device, remote_target_id: remote }) as Workspace;
    const devices = [{ id: "macbook", dials_in: true }, { id: "studio", dials_in: false }] as Device[];
    expect(readsHere(project("local", null), devices)).toBe(true);
    expect(readsHere(project("macbook", "macbook"), devices)).toBe(true);
    expect(readsHere(project("studio", "studio"), devices)).toBe(false);
    expect(readsHere(project("gone", "gone"), devices)).toBe(false);
  });
});
