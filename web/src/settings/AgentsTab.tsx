import { RefreshCwIcon, ShieldIcon } from "lucide-react";
import { useId, useState } from "react";
import type { Actions } from "../actions";
import { Disclosure, Group, Note, Row } from "../components/settings-rows";
import { Button } from "../components/ui/button";
import { ToggleGroup, ToggleGroupItem } from "../components/ui/toggle-group";
import { useInterfaceTranslation } from "../i18n/client";
import { kitUnavailableText } from "../kitWhy";
import { machineName, windowDeviceId, windowFirst } from "../screenMachine";
import { useShellStore } from "../store";
import { agentMachines, checkFailedReason, type AgentMachine } from "./agentRows";
import { AgentRow, NotInstalledRow } from "./AgentRow";
import { AgentFilesGroup } from "./AgentFilesGroup";
import { useMachineProfiles } from "./AgentProfiles";
import { IdleAgentsGroup } from "./IdleAgentsGroup";
import { HideSkillsGroup } from "./HideSkillsGroup";
import { StartingWorkGroup } from "./StartingWorkGroup";
import { useAgentsDemand } from "./useAgentsDemand";
import { coreErrorText } from "../coreError";
import { useErrorSince } from "../errorSince";

/**
 * Agents (PRD settings-cleanup D-06 to D-13): the seven supported agents of one
 * machine at a time, the machine chosen at the top when there is more than
 * one. Installed agents carry a switch and what Hide hears from their
 * sessions; the rest wait in a folded list with the link to install them.
 * Idle agents and Starting work keep their place below.
 */
export function AgentsTab({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const remote = useShellStore((s) => s.rest?.status?.remote);
  const own = useShellStore((s) => windowDeviceId(s.rest?.navigator?.devices));
  const [selected, setSelected] = useState<string | null>(null);
  useAgentsDemand(actions, true);
  // This Mac first, then the rest as the Devices tab orders them (N1).
  const machines = agentMachines(windowFirst(devices), remote);
  const machine = machines.find((row) => row.device.id === (selected ?? own)) ?? machines[0];
  return (
    <>
      {machines.length > 1 ? (
        <ToggleGroup
          type="single"
          value={machine?.device.id}
          // Choosing the chosen machine again would leave the group with no value.
          onValueChange={(value) => value && setSelected(value)}
          aria-label={t("agents.machine")}
          className="mb-lg"
          data-agents-machines="true"
        >
          {machines.map((row) => (
            <ToggleGroupItem key={row.device.id} value={row.device.id} data-agents-machine={row.device.id}>
              {machineName(devices, row.device.id, t)}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      ) : null}
      {machine ? <MachineAgents key={machine.device.id} machine={machine} actions={actions} /> : <Note>{t("agents.reading")}</Note>}
      <HideSkillsGroup />
      <IdleAgentsGroup actions={actions} />
      <StartingWorkGroup actions={actions} />
    </>
  );
}

function MachineAgents({ machine, actions }: { machine: AgentMachine; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const name = useShellStore((s) => machineName(s.rest?.navigator?.devices, machine.device.id, t));
  const { device } = machine;
  const [actedAt, setActedAt] = useState<number | null>(null);
  // A refused switch or Reinstall: the core's `kit.` error after the press.
  const failure = useErrorSince(actedAt, ["kit."]);
  const act = () => setActedAt(Date.now());
  // A machine that cannot answer keeps its profiles read-only (B30).
  const profiles = useMachineProfiles(device, machine.blocked === null, actions, name);

  // B11: a device still connecting or switched off in Devices says so and offers no retry there does not;
  // one that cannot be reached shows one line and a way to try again, not its last list.
  if (machine.blocked === "connecting" || machine.blocked === "disabled") {
    return (
      <Group>
        <Row label={<Note data-agents-blocked={`${machine.blocked}:${device.id}`}>{t(machine.blocked === "connecting" ? "agents.connecting" : "agents.disabled", { device: name })}</Note>} />
      </Group>
    );
  }
  if (machine.blocked === "unreachable") {
    return (
      <Group>
        <Row label={<Note data-agents-unreachable={device.id}>{t("agents.unreachable", { device: name })}</Note>}>
          <Button variant="secondary" size="sm" onClick={() => actions.retryDevice(device.id)} data-agents-retry={device.id}>
            {t("agents.retry")}
          </Button>
        </Row>
      </Group>
    );
  }
  // The core says when a read is under way and when the last one failed (B10); nothing here keeps time.
  const reading = device.kit?.busy === true || device.kit?.checking === true;
  const checkFailed = device.kit?.check_failed ?? null;
  return (
    <div className="@container/agents" data-agents-machine-list={device.id}>
      {machine.blocked === "unavailable" && device.kit?.unavailable ? <Note data-agents-unavailable={device.id} data-kit-unavailable={device.kit.unavailable}>{kitUnavailableText(device.kit.unavailable, t)}</Note> : null}
      {machine.unread && machine.blocked === null ? <Note data-agents-unread={device.id}>{t("agents.reading")}</Note> : null}
      {machine.unlisted && machine.blocked === null ? (
        <Group>
          <Row label={<Note data-agents-unlisted={device.id}>{t("agents.unlisted")}</Note>}>
            <Button variant="secondary" size="sm" onClick={() => actions.checkKit()} data-agents-unlisted-check={device.id}>
              {t("agents.checkAgain")}
            </Button>
          </Row>
        </Group>
      ) : null}
      {machine.installed.length + machine.notInstalled.length > 0 ? (
        <>
          <Group
            title={t("agents.installedCount", { count: machine.installed.length })}
            columns={machine.installed.length > 0 ? <ColumnHeads /> : undefined}
            note={machine.installed.length > 0 ? <PermissionScope /> : undefined}
            action={
              <Button
                variant="ghost"
                size="sm"
                disabled={reading}
                aria-label={t("agents.checkAgainAria")}
                aria-busy={reading}
                onClick={() => actions.checkKit()}
                data-agents-check={reading ? "checking" : "idle"}
              >
                <RefreshCwIcon aria-hidden="true" className={reading ? "animate-spin" : ""} />
                {reading ? t("agents.checking") : t("agents.checkAgain")}
              </Button>
            }
          >
            {machine.installed.length === 0 ? <Row label={<Note>{t("agents.noneInstalled")}</Note>} /> : null}
            {machine.installed.map((agent) => (
              <div key={agent.id} data-agent-with-profiles={agent.id}>
                <AgentRow device={device} agent={agent} actions={actions} onAct={act} />
                {profiles.inline(agent)}
              </div>
            ))}
          </Group>
          {profiles.note}
          {profiles.overlay}
          {checkFailed ? (
            <Note tone="error" data-agents-check-failed={checkFailed}>
              {t("agents.checkFailed", { reason: t(checkFailedReason(checkFailed)) })}
            </Note>
          ) : null}
          {failure ? (
            <Note tone="error" data-agents-error="true" data-core-error={failure.kind}>
              {coreErrorText(failure, t, "coreError.kit.unavailable")}
            </Note>
          ) : null}
          {machine.notInstalled.length > 0 ? (
            <Group>
              <Disclosure title={t("agents.notInstalledCount", { count: machine.notInstalled.length })} data-agents-not-installed={device.id}>
                {machine.notInstalled.map((agent) => (
                  <NotInstalledRow key={agent.id} agent={agent} />
                ))}
              </Disclosure>
            </Group>
          ) : null}
        </>
      ) : null}
      {/* The core's machine reads the operator's agent files. */}
      {device.kind !== "remote" ? <AgentFilesGroup actions={actions} /> : null}
    </div>
  );
}

/**
 * The heads of the two columns every installed row ends with, 권한 and 켬,
 * over the select and the switch. On a narrow sheet a row's controls wrap
 * under its name, so the heads would point at nothing and are left out.
 */
function ColumnHeads() {
  const { t } = useInterfaceTranslation();
  return (
    <div aria-hidden="true" className="hidden justify-end gap-sm border-x border-transparent px-md pb-xs text-caption text-muted-foreground @xl/agents:flex" data-agents-columns="true">
      <span className="w-(--size-agent-permission-w)">{t("agents.permission.label")}</span>
      <span className="w-(--size-control) text-center">{t("agents.columns.on")}</span>
    </div>
  );
}

/**
 * What an agent's 권한 covers, in a box of its own under the installed agents:
 * every agent Hide starts, an agent another agent started included, and never
 * one the operator types into a terminal, which follows the CLI's own settings.
 */
function PermissionScope() {
  const { t } = useInterfaceTranslation();
  const headingId = useId();
  return (
    <section className="flex gap-sm rounded-md bg-secondary p-md" aria-labelledby={headingId} data-agents-permission-scope="true">
      <ShieldIcon aria-hidden="true" className="mt-xxs size-(--size-icon) shrink-0 text-muted-foreground" />
      <div className="flex min-w-0 flex-col gap-xs">
        <h4 id={headingId} className="text-subhead font-semibold text-foreground">{t("agents.permission.label")}</h4>
        <p className="flex gap-sm text-body text-subtle-foreground">
          <span aria-hidden="true" className="font-mono text-success">✓</span>
          <span className="min-w-0 break-keep">{t("agents.permission.scopeHide")}</span>
        </p>
        <p className="flex gap-sm text-body text-subtle-foreground">
          <span aria-hidden="true" className="font-mono text-muted-foreground">✕</span>
          <span className="min-w-0 break-keep">{t("agents.permission.scopeTyped")}</span>
        </p>
      </div>
    </section>
  );
}
