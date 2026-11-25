import { RepeatIcon } from "lucide-react";
import { useMemo } from "react";
import type { Actions } from "../actions";
import { useAgentLogosShown } from "../agentLogoMode";
import { deviceScope } from "../agentScope";
import { LookLine } from "../automationMarks";
import { pauseReasonText } from "../automationWords";
import { AgentRowMark } from "../AgentRowMark";
import { frontDeviceId } from "../devices";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { automationAsks, automationLooks, everyDeviceLooks, type AutomationAsk, type AutomationLook } from "../ownAsks";
import { machineName } from "../screenMachine";
import type { SnapshotRest } from "../snapshot";
import { useShellStore } from "../store";
import { DeviceChip } from "./device-chip";
import { Elapsed } from "./elapsed";
import { FoldLane } from "./sidebar-agent-row";
import { SessionRowBody } from "./session-row";
import { StatusMark } from "./status-mark";
import { Button } from "./ui/button";
import { Hint } from "./ui/tooltip";

/** The automations `deviceId`'s scope names as its own Needs You items (PRD automations D-12, B23). */
export function useAutomationAsks(deviceId: string | null | undefined): AutomationAsk[] {
  const ids = useShellStore((s) => (deviceId ? deviceScope(s.rest, deviceId)?.automation_asks : undefined));
  const section = useShellStore((s) => s.automations);
  return useMemo(() => (deviceId && ids ? automationAsks(section, ids, deviceId) : []), [section, ids, deviceId]);
}

/** Every device's, the front device's first, as the sidebar's Needs You lists every device's rows. */
export function useEveryDeviceAutomationAsks(frontId: string): AutomationAsk[] {
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const section = useShellStore((s) => s.automations);
  return useMemo(() => {
    if (!devices) return [];
    const ordered = [...devices.filter((device) => device.id === frontId), ...devices.filter((device) => device.id !== frontId)];
    return ordered.flatMap((device) => automationAsks(section, device.agent_scope.automation_asks ?? [], device.id));
  }, [devices, section, frontId]);
}

/** The once automations whose run waits to be opened that `deviceId`'s scope names as its own Needs You items (mission-104 S-4). */
export function useAutomationLooks(deviceId: string | null | undefined): AutomationLook[] {
  const ids = useShellStore((s) => (deviceId ? deviceScope(s.rest, deviceId)?.automation_looks : undefined));
  const section = useShellStore((s) => s.automations);
  return useMemo(() => (deviceId && ids ? automationLooks(section, ids, deviceId) : []), [section, ids, deviceId]);
}

/** Every device's, the front device's first, as `useEveryDeviceAutomationAsks` lists the asks. */
export function useEveryDeviceAutomationLooks(frontId: string): AutomationLook[] {
  const rest = useShellStore((s) => s.rest);
  const section = useShellStore((s) => s.automations);
  return useMemo(() => everyDeviceLooks(rest, section, frontId), [rest, section, frontId]);
}

/** `useAutomationAsks` read once from a snapshot, for a count drawn outside a render's subscription. */
export function deviceAutomationAsks(rest: SnapshotRest | null, section: Parameters<typeof automationAsks>[0], deviceId: string): AutomationAsk[] {
  return automationAsks(section, deviceScope(rest, deviceId)?.automation_asks ?? [], deviceId);
}

/** `로그인 만료 · 고치고`, which [다시 켜기] follows. */
export function AutomationAskLine({ ask, className }: { ask: AutomationAsk; className?: string }) {
  const { t } = useInterfaceTranslation();
  const line = t("automationScreen.ask.line", { reason: pauseReasonText(ask.reason, t) });
  return <span className={cn("min-w-0 truncate text-caption text-warning", className)} title={line} data-automation-ask-line={ask.reason}>{line}</span>;
}

/** [다시 켜기] (B23): the line goes, and the next scheduled run runs. */
export function AutomationAskChoices({ ask, actions, className }: { ask: AutomationAsk; actions: Actions; className?: string }) {
  const { t } = useInterfaceTranslation();
  return (
    <span className={cn("flex min-w-0 flex-wrap items-center gap-xs", className)} data-automation-ask-choices={ask.automation.id}>
      <Button size="sm" variant="secondary" onClick={() => actions.automation("resume", ask.automation.id)} data-automation-ask-resume={ask.automation.id}>
        {t("automationScreen.resume")}
      </Button>
    </span>
  );
}

/** The run's answer, one quoted muted line under it (S-4); none while the core carries no answer. */
export function AutomationLookAnswer({ look, className }: { look: AutomationLook; className?: string }) {
  const answer = look.run?.answer?.trim();
  if (!answer) return null;
  return <span className={cn("min-w-0 truncate text-caption text-muted-foreground", className)} title={answer} data-automation-look-answer="true">{`"${answer}"`}</span>;
}


/** The automation mark, where an agent's logo stands on the rows beside it. */
export function AutomationMark({ className }: { className?: string }) {
  return <RepeatIcon aria-hidden="true" className={cn("shrink-0 text-muted-foreground", className)} />;
}

/** The name of the machine that runs it while another is in front, for its row's chip. */
function useRowDevice(deviceId: string): string | null {
  const { t } = useInterfaceTranslation();
  return useShellStore((s) => (deviceId === frontDeviceId(s.rest) ? null : machineName(s.rest?.navigator?.devices, deviceId, t)));
}

/**
 * An automation stopped for a person to fix, its own Needs You row (PRD
 * automations D-12, B23): `<이름>` with the Needs You mark, `<이유> ·
 * 고치고` and [다시 켜기] under it; the row opens the automation's detail. It
 * reads as a mission's own row beside it does (`SidebarMissionRow`), the
 * automation's mark in the logo's place. The 프로젝트 list's Needs You and the
 * 태스크 list's 내 차례 both draw it.
 */
export function SidebarAutomationRow({ ask, inset, actions, ...data }: { ask: AutomationAsk; inset: string; actions: Actions } & Record<`data-${string}`, string>) {
  const { t } = useInterfaceTranslation();
  const logos = useAgentLogosShown();
  const device = useRowDevice(ask.device_id);
  const reason = pauseReasonText(ask.reason, t);
  const name = [ask.automation.name, device, t("automationScreen.title"), reason].filter(Boolean).join(", ");
  return (
    <li data-automation-ask={ask.automation.id} data-automation-ask-reason={ask.reason} className="group/row relative flex items-start gap-xs rounded-sm py-xs pr-xs text-body hover:bg-accent" style={{ paddingLeft: inset }}>
      <Hint label={name} reveals>
        <Button
          variant="text"
          size="inline"
          aria-label={name}
          data-automation-ask-open={ask.automation.id}
          {...data}
          className="absolute inset-0 rounded-sm focus-visible:ring-inset"
          onClick={() => actions.openAutomation(ask.automation.id)}
        />
      </Hint>
      <SessionRowBody
        markClassName="pointer-events-none"
        mark={<>
          <StatusMark mark="turn" />
          {logos ? <AutomationMark className="size-(--size-agent-badge-compact)" /> : null}
        </>}
        head={<>
          <span aria-hidden="true" className="pointer-events-none min-w-0 flex-1 truncate text-foreground">{ask.automation.name}</span>
          {device ? <DeviceChip label={device} className="pointer-events-none max-w-2/5" /> : null}
          <Elapsed since={ask.since_unix_ms} aria-hidden="true" className="pointer-events-none shrink-0 font-mono text-caption text-muted-foreground" data-automation-ask-elapsed="true" />
          <FoldLane />
        </>}
      >
        <span aria-hidden="true" className="pointer-events-none flex min-w-0 items-baseline gap-xs leading-(--size-sidebar-line-detail)"><AutomationAskLine ask={ask} /></span>
        <AutomationAskChoices ask={ask} actions={actions} className="relative z-10 mt-xs" />
      </SessionRowBody>
    </li>
  );
}

/**
 * A once automation whose run ended, its own Needs You row (mission-104 S-4):
 * `<이름>` with the Needs You badge carrying a clock, `자동화 · 한 번 · ✓ 끝남
 * · 4분` under it and the run's answer quoted; the row opens that run in the
 * automation screen, which puts it away. The 프로젝트 list's Needs You and the
 * 태스크 list's 내 차례 both draw it, after the automations stopped for a person.
 */
export function SidebarAutomationLookRow({ look, inset, actions, ...data }: { look: AutomationLook; inset: string; actions: Actions } & Record<`data-${string}`, string>) {
  const { t } = useInterfaceTranslation();
  const device = useRowDevice(look.device_id);
  const name = [look.automation.name, device, t("automationScreen.look.kind")].filter(Boolean).join(", ");
  return (
    <li data-automation-look={look.automation.id} data-automation-look-run={look.run?.number} className="group/row relative flex items-start gap-xs rounded-sm py-xs pr-xs text-body hover:bg-accent" style={{ paddingLeft: inset }}>
      <Hint label={name} reveals>
        <Button
          variant="text"
          size="inline"
          aria-label={name}
          data-automation-look-open={look.automation.id}
          {...data}
          className="absolute inset-0 rounded-sm focus-visible:ring-inset"
          onClick={() => actions.openAutomation(look.automation.id, look.automation.look_run ?? null)}
        />
      </Hint>
      <SessionRowBody
        markClassName="pointer-events-none"
        mark={<>
          <StatusMark mark="turn" automation="once" />
          <AgentRowMark kind={look.automation.agent.kind} />
        </>}
        head={<>
          <span aria-hidden="true" className="pointer-events-none min-w-0 flex-1 truncate text-foreground">{look.automation.name}</span>
          {device ? <DeviceChip label={device} className="pointer-events-none max-w-2/5" /> : null}
          {look.since_unix_ms > 0 ? <Elapsed since={look.since_unix_ms} aria-hidden="true" className="pointer-events-none shrink-0 font-mono text-caption text-muted-foreground" data-automation-look-elapsed="true" /> : null}
          <FoldLane />
        </>}
      >
        <span aria-hidden="true" className="pointer-events-none flex min-w-0 items-baseline gap-xs leading-(--size-sidebar-line-detail)"><LookLine run={look.run} /></span>
        <span aria-hidden="true" className="pointer-events-none flex min-w-0 leading-(--size-sidebar-line-detail)"><AutomationLookAnswer look={look} /></span>
      </SessionRowBody>
    </li>
  );
}

