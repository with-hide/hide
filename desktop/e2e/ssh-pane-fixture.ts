// A shell on either side of the isolated SSH fixture, created through that
// device's connected node with the same Add project event the UI sends.
import { expect } from "@playwright/test";
import fs from "node:fs";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { startEngine, type EngineServer } from "../../web/e2e/engine-fixture";
import { connectStateDir, type HidedControl } from "../../web/e2e/hided-control.mjs";
import { openProject, sendKeys, viewPane, waitForPaneInput } from "../../web/e2e/daemon-actions";
import { endPaneHolders, ownedHolders } from "../../web/e2e/pane-holders.mjs";
import { deviceHome, deviceState } from "./device-home";
import type { Isolated } from "./fixture";

const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;

/** Only the snapshot fields this fixture observes; desktop does not import
 * the web renderer and its JSX dependency graph for control-only fixtures. */
export type SshRest = {
  navigator?: { focused_device_id?: string };
  focused?: { pane_id?: string };
  status?: { remote?: { target_id: string; state: string; session?: {
    workspaces: { id: string; path: string }[];
    pane_layouts: { workspace_id: string; frames: { pane_id: string }[] }[];
    agents: { pane_id: string; session_id?: string; state: { needs_you: boolean } }[];
  } }[] };
};

export type SshPaneFixture = Awaited<ReturnType<typeof startSshPaneFixture>>;

export async function startSshPaneFixture() {
  const server: EngineServer = await startEngine();
  let control: HidedControl | null = null;
  let remoteState: string | null = null;
  let machine: string | undefined;
  let panes: string[] = [];
  server.afterStop?.(() => {
    control?.close();
    if (remoteState) endPaneHolders(remoteState);
  });
  async function openOwnProject(folder: string, label: string): Promise<string[]> {
    if (!control) throw new Error("attach the SSH fixture before opening a project");
    if (!machine) return [(await openProject(control, folder, label)).pane];
    folder = fs.realpathSync(folder);
    control.send("create_workspace", { device_id: machine, path: folder, label, initialize_git: false, confirmed_outside_home: folder });
    return control.until((rest) => {
      const session = (rest as SshRest).status?.remote?.find((row) => row.target_id === machine)?.session;
      const workspace = session?.workspaces.find((row) => row.path === folder);
      const layout = session?.pane_layouts.find((row) => row.workspace_id === workspace?.id);
      return layout?.frames.length ? layout.frames.map((frame) => frame.pane_id) : undefined;
    }, `the project ${label} on ${machine}`, 60_000);
  }
  return {
    ...server,
    async open(folder: string, label: string): Promise<void> {
      await openOwnProject(folder, label);
    },
    get panes() { return panes; },
    get control() { if (!control) throw new Error("attach the SSH fixture first"); return control; },
    /** After the daemon is healthy, and on a device after its registration connected. */
    async attach(run: Isolated, device?: string, home?: string): Promise<void> {
      if (control) throw new Error("the SSH fixture is already attached");
      control = await connectStateDir(run.env.HIDE_STATE_DIR!);
      machine = device;
      if (!device) {
        panes = await openOwnProject(path.join(server.root, "fixture"), "fixture");
      } else {
        remoteState = deviceState(home ?? deviceHome());
        const before = await control.until((rest) => ({ device: (rest as SshRest).navigator?.focused_device_id }), "the device in front");
        await control.until((rest) => (rest as SshRest).status?.remote?.some((row) => row.target_id === device && row.state === "connected"), `the device ${device} connected`, 120_000);
        panes = await openOwnProject(path.join(server.root, "fixture"), "fixture");
        if (before.device) {
          control.send("focus_device", { device_id: before.device });
          await control.until((rest) => (rest as SshRest).navigator?.focused_device_id === before.device, "the device in front restored");
        }
      }
      viewPane(control, panes[0]!);
      await waitForPaneInput(control, panes[0]!);
    },
    /** The core routes a fully scoped device pane. */
    async send(text: string, pane = panes[0]): Promise<void> {
      if (!pane) throw new Error("attach the SSH fixture before sending input");
      if (!control) throw new Error("the SSH fixture has no daemon connection");
      viewPane(control, pane);
      await waitForPaneInput(control, pane);
      sendKeys(control, pane, text);
    },
    /** Run in the real pane, with its ancestry, without letting synthetic keys
     * change which device the command observes in front. The gate is bounded
     * and cancelled on setup failure; no command survives its test indefinitely. */
    async command(line: string, label: string): Promise<{ status: number; stdout: string }> {
      const output = path.join(server.root, `${label}.out`);
      const status = path.join(server.root, `${label}.exit`);
      const gate = path.join(server.root, `${label}.gate`);
      const arrived = `${gate}.arrived`;
      for (const file of [output, status, gate, arrived]) fs.rmSync(file, { force: true });
      const command = `${line} > ${quote(output)}; printf '%s' "$?" > ${quote(status)}\n`;
      if (!control) throw new Error("attach the SSH fixture before running a command");
      const before = await control.until((rest) => ({
        device: (rest as SshRest).navigator?.focused_device_id,
        pane: (rest as SshRest).focused?.pane_id,
      }), "the screen's location before fixture input");
      // The POSIX shell gate needs no device Node/Python installation.
      const wait = `printf ready > ${quote(arrived)}; ssh_fixture_wait=0; while [ ! -e ${quote(gate)} ] && [ "$ssh_fixture_wait" -lt 200 ]; do sleep 0.1; ssh_fixture_wait=$((ssh_fixture_wait + 1)); done; `;
      let restored = false;
      try {
        viewPane(control, panes[0]!);
        await waitForPaneInput(control, panes[0]!);
        sendKeys(control, panes[0]!, `${wait}if [ -e ${quote(gate)} ] && [ "$(cat ${quote(gate)})" = run ]; then ${command.trimEnd()}; else printf 9 > ${quote(status)}; fi\r`);
        await expect.poll(() => fs.existsSync(arrived), { timeout: 20_000 }).toBe(true);
        if (before.device) control.send("focus_device", { device_id: before.device });
        if (before.pane && !machine) control.send("focus_pane", { pane_id: before.pane, origin: "operator" });
        await control.until((rest) => !before.device || (rest as SshRest).navigator?.focused_device_id === before.device, "the original device restored");
        restored = true;
      } finally {
        fs.writeFileSync(gate, restored ? "run" : "cancel");
      }
      await expect.poll(() => fs.existsSync(status), { timeout: 30_000 }).toBe(true);
      return { status: Number(fs.readFileSync(status, "utf8")), stdout: fs.readFileSync(output, "utf8") };
    },
  };
}

/** The actual process under a test-owned holder, including its live cwd.
 * Listing a projected row alone would not prove that the remote start ran. */
export function ownSshPaneRunning(home: string, device: string, text: string): { pane: string; info: string; cwd?: string } | null {
  const table = execFileSync("ps", ["-Ao", "pid=,ppid=,command="], { encoding: "utf8", timeout: 5_000 })
    .trim().split("\n").map((line) => {
      const match = line.trim().match(/^(\d+)\s+(\d+)\s+(.+)$/);
      if (!match) throw new Error("unreadable fixture process row");
      return { pid: Number(match[1]), parent: Number(match[2]), command: match[3]! };
    });
  for (const holder of ownedHolders(deviceState(home))) {
    if (holder.program === undefined) continue; // Reserved holder, no program yet.
    const owned = new Set<number>([holder.program]);
    for (let count = -1; count !== owned.size;) {
      count = owned.size;
      for (const process of table) if (owned.has(process.parent)) owned.add(process.pid);
    }
    const running = table.find((row) => owned.has(row.pid) && row.command.includes(text) && /^(?:[^ ]*\/)?claude(?: |$)/.test(row.command));
    if (!running) continue;
    const record = JSON.parse(fs.readFileSync(holder.file, "utf8")) as { pane: string };
    const cwd = process.platform === "linux"
      ? fs.readlinkSync(`/proc/${running.pid}/cwd`)
      : execFileSync("lsof", ["-a", "-p", String(running.pid), "-d", "cwd", "-Fn"], { encoding: "utf8", timeout: 5_000 })
        .split("\n").find((line) => line.startsWith("n"))?.slice(1);
    if (!cwd) throw new Error(`no live cwd for fixture process ${running.pid}`);
    return { pane: `remote:${device}:pane:${record.pane.replaceAll("_", ":")}`, info: running.command, cwd };
  }
  return null;
}

