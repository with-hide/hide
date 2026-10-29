// `hide device` from a pane on an isolated pinned Herdr and hided (PRD
// system-ssh-device-link B29-B31): an agent reads what a device waits on from
// the operator, its retry starts nothing the operator has not allowed, and
// `retry --wait` answers the attempt's failure by the row's action. The
// device is a Host of the fixture's ssh config that never resolves, so the
// system ssh fails to find it; a refusal from a device's own pane needs a
// connected device and is the core's test (`ssh_sign_in.rs`, `devices.rs`).

import { expect, test } from "@playwright/test";
import path from "node:path";
import { fixtureExecutable } from "./platform-fixture";
import { runInPane, startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, writeSshHost, type Daemon } from "./hided-fixture";
import { enterWorkspace } from "./wire";

test.describe.configure({ timeout: 120_000 });

const ALIAS = "unreachable-e2e";

type Device = {
  id: string;
  state: string;
  waits_on: { kind: string } | null;
  failure: { action: string; cause: string | null; detail: string | null } | null;
  next_action: string | null;
};
type Answer = { ok: boolean; value: { device: Device; started?: boolean; devices?: Device[] }; error?: { code: string } };

async function fromPane(herdr: HerdrFixture, daemon: Daemon, args: string[], sequence: number): Promise<Answer> {
  const hide = path.resolve("..", "target", "debug", fixtureExecutable("hide"));
  const ran = await runInPane(herdr, herdr.panes[0], `device-${sequence}`, { env: { HIDE_STATE_DIR: daemon.stateDir }, argv: [hide, "device", ...args, "--json"], stdout: true });
  expect(ran.status, `${ran.stdout} ${ran.stderr}`).toBe(0);
  return JSON.parse(ran.stdout) as Answer;
}

test("an agent reads what a device waits on and retries it until the attempt's outcome", async ({ page }) => {
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "device-cli");
    writeSshHost(daemon, ALIAS);
    // A pane's command needs its checkout registered, which entering the Workspace waits for.
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");

    // B29, B31: an added device waits on the operator's Allow, and says what to ask them.
    const added = await fromPane(herdr, daemon, ["add", ALIAS], 1);
    expect(added.value.device).toMatchObject({ id: ALIAS, state: "operator_turn", waits_on: { kind: "allow_and_install" }, failure: null });
    expect(Object.keys(added.value.device).sort()).toEqual(["failure", "id", "label", "next_action", "ssh_alias", "state", "waits_on"]);

    // B30, D-11: a retry does not stand in for the Allow; it starts nothing and answers at once.
    const notAllowed = await fromPane(herdr, daemon, ["retry", ALIAS, "--wait"], 2);
    expect(notAllowed.value.started).toBe(false);
    expect(notAllowed.value.device.state).toBe("operator_turn");
    expect(notAllowed.value.device.next_action).toContain("Allow and install");

    // The operator allows the helper on the row.
    await page.locator("[data-open-settings]").click();
    await page.locator('[data-settings-tab="devices"]').click();
    await page.locator(`[data-device-host-allow-inline="${ALIAS}"]`).click();
    await page.locator('[data-device-host-allow-go="true"]').click();

    // B30, operator decision 2026-10-30: the wait ends on the attempt's failure, read as the row's
    // action for it with the core's cause beside it: a name ssh cannot find is fixed, then retried.
    const failed = await fromPane(herdr, daemon, ["retry", ALIAS, "--wait"], 3);
    expect(failed.value.device).toMatchObject({ state: "failed", waits_on: null, failure: { action: "retry", cause: "name_not_found" } });
    expect(failed.value.device.next_action).toContain(`hide device retry ${ALIAS}`);

    const list = await fromPane(herdr, daemon, ["list"], 4);
    expect(list.value.devices?.map((device) => [device.id, device.state, device.failure?.action])).toEqual([[ALIAS, "failed", "retry"]]);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
