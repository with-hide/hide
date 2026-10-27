// The first-run wizard (PRD first-run-onboarding D-01): 1 agents, 2
// saved devices when present, notifications, the first project, then the start panel. The step on
// screen is the core's (`ui_state.first_run`), so a reload or a restart
// reopens the step the operator was on, and nothing closes it but the steps'
// own buttons (B5, B6). A browser tab shows the first step alone (B26).
// Help > Set up Hide reopens the same steps from the first, as this page's
// own walk that closes with Escape and changes only what a step's button
// sends (B45, B46).
//
// The layout is the operator-approved Pen design (D-20, `Screen / Onboarding`):
// a dialog over the dimmed shell, the three-part progress above the title,
// the step's main button at the footer's right with its ⌘↵ keycap.

import { CheckIcon, ChevronRightIcon, FolderPlusIcon, GraduationCapIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ComponentProps, type KeyboardEvent, type ReactNode } from "react";
import { create } from "zustand";
import type { TFunction } from "i18next";
import type { Actions } from "./actions";
import { AddProjectViews } from "./AddProjectDialog";
import { agentLogo, monogram } from "./agentLogos";
import { selection, tileSwitchable } from "./agentOnboardingRules";
import { Note, Status } from "./components/settings-rows";
import { Button } from "./components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "./components/ui/dialog";
import { Kbd } from "./components/ui/kbd";
import { accountHome, failedAgents, firstRunStep, type FailedAgent, firstStartAgent, noAgentInstalled, SUGGESTED_INSTALLS, tileStatus, whatChanges, wizardSteps, type TileStatus, type WizardStep } from "./firstRun";
import { FirstRunDevices } from "./FirstRunDevices";
import { FirstRunHerdr } from "./HerdrTakeover";
import { herdrStepShown } from "./herdrTakeoverRules";
import { firstRunAgent } from "./hideAi";
import { BrandOwl } from "./components/brand-owl";
import { hostBridge, keySystem } from "./host";
import { useInterfaceTranslation } from "./i18n/client";
import type { MessageKey } from "./i18n/catalogs";
import { cn } from "./lib/utils";
import { NotificationLine, useNotificationControl, usePermission } from "./NotificationControls";
import { kitUnavailableText, kitWhyText } from "./kitWhy";
import { kitRefusalLine } from "./settings";
import { FIRST_CHAPTER, openPractice } from "./practice/channel";
import { NotOnSystem } from "./settings/AgentRow";
import { installDocUrl, machineSystem, supportedAgents, type AgentPart } from "./settings/agentRows";
import { useAgentsDemand } from "./settings/useAgentsDemand";
import { localDeviceId, type KitAgent, type WorkspaceRegistration } from "./snapshot";
import { useStartPanel } from "./startDraft";
import { useShellStore } from "./store";
import { fieldLabel } from "./shortcutLabels";
import { holdsFieldModifier } from "./shortcuts";
import { useUiStore } from "./ui";

/** Help > Set up Hide's walk: this page's own, gone with a close (B46). */
type SetupWalk = { step: WizardStep | null; open: () => void; go: (step: WizardStep) => void; close: () => void };

export const useSetupWalk = create<SetupWalk>((set) => ({
  step: null,
  // It opens on moving the operator's older Herdr only while there is one to move (PRD herdr-takeover B17, D-17).
  open: () => set({ step: herdrStepShown(useShellStore.getState().rest?.status?.herdr_takeover) ? "herdr" : "agents" }),
  go: (step) => set({ step }),
  close: () => set({ step: null }),
}));

const NO_REGISTRATIONS: WorkspaceRegistration[] = [];

export function FirstRunWizard({ actions }: { actions: Actions }) {
  const desktop = hostBridge() !== null;
  const firstRun = useShellStore((s) => firstRunStep(s.rest, desktop));
  const setup = useSetupWalk((s) => s.step);
  const kit = useShellStore((s) => s.rest?.navigator?.devices?.find((device) => device.kind !== "remote")?.kit);
  const takeover = useShellStore((s) => s.rest?.status?.herdr_takeover);
  // The first run owns the screen; Help's walk waits behind it.
  const step = firstRun ?? setup;
  // A walk on a Herdr step with nothing left to move goes on to the agents; the core moves the first run itself.
  useEffect(() => {
    if (firstRun === null && setup === "herdr" && !herdrStepShown(takeover)) useSetupWalk.getState().go("agents");
  }, [firstRun, setup, takeover]);
  if (step === null) return null;
  // The core's Herdr step waits for its look at the server; there is nothing to draw until it answers.
  if (step === "herdr" && !herdrStepShown(takeover)) return null;
  // The agent step draws this Mac's kit, which lists every supported agent once it has been read;
  // a kit that could not run, or a pass that stopped before changing anything, stays on screen to say why.
  if (step === "agents" && !kit?.agents?.length && !kit?.unavailable && !kit?.refused) return null;
  return <Wizard actions={actions} step={step} reopened={firstRun === null} desktop={desktop} />;
}

function Wizard({ actions, step, reopened, desktop }: { actions: Actions; step: WizardStep; reopened: boolean; desktop: boolean }) {
  const { t } = useInterfaceTranslation();
  const profiles = useShellStore((s) => s.rest?.status?.herdr_profiles);
  const saved = useShellStore((s) => s.rest?.ui_state?.first_run_choices);
  const [chosen, setChosen] = useState<readonly string[] | null>(() => reopened ? null : saved?.agents ?? null);
  const [answered, setAnswered] = useState(false);
  const takeover = useShellStore((s) => s.rest?.status?.herdr_takeover ?? null);
  const steps = wizardSteps(profiles, step === "herdr" || (desktop && herdrStepShown(takeover)));
  // Read saved configuration once per walk. This does not connect a device.
  useEffect(() => { if (desktop) actions.listHerdrProfiles(); }, [actions, desktop]);
  const primary = useRef<(() => void) | null>(null);
  // Each render's step names its own main button; a step without one (the project step) leaves Enter to its controls.
  primary.current = null;
  const close = () => useSetupWalk.getState().close();
  const number = Math.max(steps.indexOf(step), 0) + 1;
  const back = step === "agents" || step === "herdr" ? undefined : () => {
    if (reopened) useSetupWalk.getState().go(steps[Math.max(steps.indexOf(step) - 1, 0)]!);
    else actions.backFirstRun(step);
  };
  const nextAgents = () => useSetupWalk.getState().go(desktop && steps.includes("devices") ? "devices" : "notifications");
  const choose = (agents: readonly string[]) => {
    setChosen(agents);
    actions.saveFirstRunDraft({ agents: [...agents] });
  };
  // Enter on the step itself, or ⌘↵ (Ctrl+Enter off macOS) anywhere in it, presses its main button; a control with its own Enter keeps it (B40).
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key !== "Enter" || event.nativeEvent.isComposing) return;
    const target = event.target as HTMLElement;
    if (!holdsFieldModifier(event, keySystem()) && target.closest("button, a, input, textarea, select, [role=switch], [role=combobox]")) return;
    if (!primary.current) return;
    event.preventDefault();
    primary.current();
  };
  // Each step starts with the keyboard on the dialog itself, so Enter is the step's main button and
  // not the first link (B40); a step that replaces the one holding the keyboard takes it back.
  useEffect(() => {
    const surface = document.querySelector<HTMLElement>("[data-first-run]");
    if (surface && !surface.contains(document.activeElement)) surface.focus();
  }, [step]);
  return (
    <Dialog open onOpenChange={(open) => { if (!open && reopened) close(); }}>
      <DialogContent
        initialFocus="container"
        data-first-run={step}
        data-first-run-mode={reopened ? "setup" : "first_run"}
        showCloseButton={reopened}
        aria-describedby={undefined}
        // The step's own header sits a small gap under the progress line.
        className="w-(--size-onboarding-dialog-w) [&_[data-slot=dialog-header]]:gap-sm [&_[data-slot=dialog-header]]:pt-sm [&_[data-slot=dialog-title]]:text-headline"
        onKeyDown={onKeyDown}
        // The first run closes only through its steps (B5).
        onEscapeKeyDown={(event) => { if (!reopened) event.preventDefault(); }}
        onPointerDownOutside={(event) => event.preventDefault()}
        onInteractOutside={(event) => event.preventDefault()}
      >
          <div className="flex shrink-0 items-center gap-sm px-lg pt-lg pr-xxl">
        {desktop ? (
          <div className="flex flex-wrap items-center gap-sm" data-first-run-progress={number}>
            <div className="flex gap-xs" aria-hidden="true">
              {steps.map((row, index) => (
                <span key={row} className={cn("h-xs w-xl rounded-full", index < number ? "bg-primary" : "bg-secondary")} />
              ))}
            </div>
            <span className="text-caption text-muted-foreground">{t("firstRun.progress", { step: number, total: steps.length })}</span>
          </div>
        ) : null}
            <BrandOwl key={step} pose={step === "agents" ? "agents" : step === "notifications" || step === "herdr" ? "devices" : "ai"} className="ml-auto h-(--size-owl-onboarding) w-(--size-owl-onboarding)" />
          </div>
        {step === "herdr" && takeover ? (
          <FirstRunHerdr
            actions={actions}
            takeover={takeover}
            primary={primary}
            // Help's walk moves on its own; the first run's Not now is the core's (D-16, D-29).
            onLater={() => {
              if (!reopened) actions.herdrTakeover("later");
              else if (takeover.same_protocol) useSetupWalk.getState().go("agents");
              else close();
            }}
            onMoved={() => {
              actions.herdrTakeover("continue");
              if (reopened) useSetupWalk.getState().go("agents");
            }}
          />
        ) : step === "agents" ? (
          <AgentsStep actions={actions} reopened={reopened} desktop={desktop} primary={primary} chosen={chosen} onChoose={choose} onNext={nextAgents} />
        ) : step === "devices" ? (
          <FirstRunDevices actions={actions} primary={primary} onBack={back!} onNext={() => reopened ? useSetupWalk.getState().go("notifications") : actions.advanceFirstRun("devices")} />
        ) : step === "notifications" ? (
          <NotificationsStep actions={actions} reopened={reopened} primary={primary} onBack={back!} answered={answered || saved?.notifications_answered === true} onAnswered={() => setAnswered(true)} />
        ) : (
          <StartStep actions={actions} reopened={reopened} onBack={back!} />
        )}
      </DialogContent>
    </Dialog>
  );
}

/** A step's main button with the keycap that presses it from anywhere in the step. */
function MainButton({ variant = "default", ...props }: ComponentProps<typeof Button>) {
  return (
    <span className="flex items-center gap-sm">
      <Button variant={variant} {...props} />
      <Kbd>{fieldLabel("Enter")}</Kbd>
    </span>
  );
}

type StepProps = { actions: Actions; reopened: boolean; primary: { current: (() => void) | null } };

function AgentsStep({ actions, reopened, desktop, primary, chosen, onChoose, onNext }: StepProps & { desktop: boolean; chosen: readonly string[] | null; onChoose: (agents: readonly string[]) => void; onNext: () => void }) {
  const { t } = useInterfaceTranslation();
  const kit = useShellStore((s) => s.rest?.navigator?.devices?.find((device) => device.kind !== "remote")?.kit ?? null);
  const agents = useMemo(() => supportedAgents(kit?.agents), [kit?.agents]);
  const providers = useShellStore((s) => s.rest?.status?.background_ai?.providers);
  const ai = useShellStore((s) => s.rest?.status?.background_ai);
  const answered = useShellStore((s) => s.rest?.ui_state?.agent_onboarding === "done");
  const [showChanges, setShowChanges] = useState(false);
  // The agents the last Continue sent: a failure names those (B12).
  const [sent, setSent] = useState<string[] | null>(null);
  // The kit is busy from the snapshot that takes Continue until its pass lands
  // (B10); the press itself holds the button only until that next snapshot.
  // A second press meanwhile is the same intent, which the core ignores.
  const busy = kit?.busy ?? false;
  const revision = useShellStore((s) => s.revision);
  const [pressedAt, setPressedAt] = useState<number | null>(null);
  const pressed = pressedAt !== null && pressedAt === revision;
  const on = chosen === null ? selection(agents, new Set(), reopened || answered ? "enabled" : "installed") : new Set(agents.filter((agent) => tileSwitchable(agent) && chosen.includes(agent.id)).map((agent) => agent.id));
  // The sign-in probe runs only while a page asks for it.
  useAgentsDemand(actions, false);
  const hideAi = firstRunAgent(ai, on);
  // This Mac's account home, the folder its Hide Home sits in: What changes writes it as `~` (D-26).
  const home = useShellStore((s) => accountHome(s.rest?.ui_state?.workspace_registrations?.find((row) => row.home && row.device_id === localDeviceId(s.rest))?.path));
  const changes = whatChanges(agents, on, kit, home);
  const applying = pressed || busy;
  const failures = sent && !applying ? failedAgents(agents, sent) : [];
  // The kit could not run here, whether or not Continue was pressed: the step says why, and Try again
  // only has the core read the agents again; the choice is sent once they are back.
  const unavailable = !applying ? (kit?.unavailable ?? null) : null;
  // The pass stopped before changing anything: the step says what stopped it, and Try again runs the
  // same pass again, which checks again; nothing is chosen until it runs and the tiles are back.
  const refused = !applying ? (kit?.refused ?? null) : null;
  const toggle = (id: string) => {
    const next = new Set(on);
    if (!next.delete(id)) next.add(id);
    onChoose(agents.filter((agent) => next.has(agent.id)).map((agent) => agent.id));
  };
  const proceed = () => {
    if (applying || (agents.length === 0 && !unavailable && !refused)) return;
    const picked = agents.filter((agent) => on.has(agent.id)).map((agent) => agent.id);
    const unchanged = !unavailable && !refused && agents.every((agent) => !tileSwitchable(agent) || (
      agent.enabled === on.has(agent.id) && (!agent.enabled || [agent.skill, agent.hook, agent.herdr].every((piece) => !piece || !["outdated", "not_installed", "removed", "failed"].includes(piece.state)))
    ));
    // The core queues no work for a healthy unchanged Help choice, so there
    // will be no later kit report to wait for.
    if (reopened && unchanged) {
      actions.applyAgentOnboarding(picked, !desktop && !reopened);
      onNext();
      return;
    }
    setSent(picked);
    setPressedAt(revision);
    // A browser tab's wizard is this step alone, so it ends with it (B26).
    actions.applyAgentOnboarding(picked, !desktop && !reopened);
  };
  // Help waits for the same kit result as first run; an old completion never
  // navigates another step. An unchanged, healthy choice queues no install.
  useEffect(() => {
    if (!reopened || sent === null || applying || unavailable || refused || failures.length > 0) return;
    onNext();
  }, [reopened, sent, applying, unavailable, refused, failures.length]);
  primary.current = proceed;
  const label = applying ? t("firstRun.agents.applying") : failures.length > 0 || unavailable || refused ? t("firstRun.agents.retry") : t("firstRun.agents.continue");
  return (
    <>
      <DialogHeader>
        <DialogTitle>{t("firstRun.agents.title")}</DialogTitle>
        <DialogDescription>{t("firstRun.agents.consent")}</DialogDescription>
      </DialogHeader>
      <DialogBody className="flex flex-col gap-md" data-agent-onboarding="true">
        {noAgentInstalled(agents) ? (
          <p className="flex flex-wrap items-baseline gap-x-md gap-y-xs" data-first-run-no-agent="true">
            <Status tone="muted">{t("firstRun.agents.noneInstalled")}</Status>
            {SUGGESTED_INSTALLS.map((id) => (
              <a key={id} href={installDocUrl(id) ?? undefined} target="_blank" rel="noreferrer" className="text-body font-medium text-foreground hover:underline" data-first-run-install={id}>
                {t("firstRun.agents.install", { agent: agents.find((agent) => agent.id === id)?.label ?? id })}
              </a>
            ))}
          </p>
        ) : null}
        <div role="group" aria-label={t("onboarding.grid")} className="grid grid-cols-4 gap-sm">
          {agents.map((agent) => (
            <AgentTile key={agent.id} agent={agent} on={on.has(agent.id)} status={tileStatus(agent, providers)} disabled={applying} onToggle={() => toggle(agent.id)} />
          ))}
        </div>
        <div className="flex flex-col gap-xs">
          <button type="button" aria-expanded={showChanges} onClick={() => setShowChanges((open) => !open)} className="flex items-center gap-xs self-start rounded-xs text-body font-medium text-subtle-foreground outline-none focus-visible:ring-1 focus-visible:ring-ring" data-first-run-what-changes="true">
            <ChevronRightIcon aria-hidden="true" className={cn("size-(--size-icon) text-muted-foreground transition-transform", showChanges && "rotate-90")} />
            {t("firstRun.agents.whatChanges")}
          </button>
          {showChanges ? (
            <ul className="flex flex-col rounded-md bg-card px-md py-xs text-caption" data-first-run-changes="true">
              {changes.rows.map((row) => (
                <li key={row.path} className="flex min-h-(--size-sidebar-line) items-center gap-md" data-first-run-change={row.path}>
                  <span className="min-w-0 flex-1 break-all font-mono text-foreground">{row.path}</span>
                  <span className="shrink-0 text-muted-foreground">{row.labels.join(", ")} · {t(AGENT_PART[row.part])}</span>
                </li>
              ))}
              {changes.rows.length === 0 ? <li className="flex min-h-(--size-sidebar-line) items-center text-muted-foreground">{t("firstRun.agents.nothingChanges")}</li> : null}
              {changes.cli ? (
                <li className="flex min-h-(--size-sidebar-line) items-center gap-md" data-first-run-change="cli">
                  <span className="min-w-0 flex-1 break-all font-mono text-foreground">{changes.cli}</span>
                  <span className="shrink-0 text-muted-foreground">{t("firstRun.agents.cli")}</span>
                </li>
              ) : null}
            </ul>
          ) : null}
        </div>
        <p className="text-caption text-muted-foreground">{t("onboarding.devices")}</p>
        {unavailable ? (
          <Note tone="error" data-first-run-unavailable="true">
            <Status tone="error">{t("firstRun.agents.unavailable", { reason: kitUnavailableText(unavailable, t) })}</Status>
          </Note>
        ) : null}
        {refused ? (
          <Note tone="error" data-first-run-refused={refused.cause}>
            <Status tone="error">{kitRefusalLine(refused, t)}</Status>
          </Note>
        ) : null}
        {failures.map((failure) => (
          <Note key={failure.id} tone="error" data-first-run-failed={failure.id}>
            <Status tone="error">{failureLine(failure, t)}</Status>
          </Note>
        ))}
      </DialogBody>
      <DialogFooter>
        {/* The agent Hide AI will run on under its first-run rule; no line when none of the chosen agents is signed in (B13). */}
        <span className="min-w-0 flex-1 text-caption text-muted-foreground">
          {hideAi ? <span data-onboarding-hide-ai={hideAi.id}>{t("hideAi.firstRun", { agent: hideAi.label })}</span> : null}
        </span>
        {/* A refused pass may take a terminal and a restart to clear, so the step can be left; the question stays unanswered. */}
        {refused && !reopened ? (
          <Button variant="ghost" onClick={() => actions.advanceFirstRun("agents")} data-first-run-refused-close="true">
            {t("firstRun.notifications.notNow")}
          </Button>
        ) : null}
        {/* With no agent read yet there is no choice to send; the next read brings the tiles back. */}
        <MainButton onClick={proceed} disabled={applying || (agents.length === 0 && !unavailable && !refused)} aria-busy={applying} data-onboarding-apply="true">
          {label}
        </MainButton>
      </DialogFooter>
    </>
  );
}

/** What each piece is called under What changes: the words Settings > Agents uses. */
const AGENT_PART: Record<AgentPart, MessageKey> = {
  hook: "agents.part.hook",
  plugin: "agents.part.plugin",
  extension: "agents.part.extension",
  skill: "agents.part.skill",
  herdr: "agents.part.herdr",
};

const TILE_STATUS: Record<Exclude<TileStatus, "unsupported">, MessageKey> = {
  checking: "firstRun.tile.checking",
  ready: "firstRun.tile.ready",
  not_signed_in: "firstRun.tile.notSignedIn",
  installed: "firstRun.tile.installed",
  not_installed: "firstRun.tile.notInstalled",
};

/**
 * One agent: its mark, its name and what Hide knows of it (B8). An agent
 * installed here is a switch; one that is not is dimmed and links its
 * official install page instead (B9).
 */
function AgentTile({ agent, on, status, disabled, onToggle }: { agent: KitAgent; on: boolean; status: TileStatus; disabled: boolean; onToggle: () => void }) {
  const { t } = useInterfaceTranslation();
  const switchable = tileSwitchable(agent);
  const logo = agentLogo(agent.id);
  // An agent installed on a system Hide cannot attach to yet has nothing to install (B49).
  const install = switchable || status === "unsupported" ? null : installDocUrl(agent.id);
  // The mark and the name dim; the line that says why stays legible.
  const dim = switchable ? "" : "opacity-(--opacity-dimmed)";
  const line =
    status === "ready" ? (
      <Status tone="ok">{t(TILE_STATUS.ready)}</Status>
    ) : status === "not_signed_in" ? (
      <Status tone="warn">{t(TILE_STATUS.not_signed_in)}</Status>
    ) : status === "unsupported" ? (
      <NotOnSystem system={machineSystem(null)} />
    ) : (
      t(TILE_STATUS[status])
    );
  const body: ReactNode = (
    <>
      <span className={cn("flex size-(--size-agent-logo) shrink-0 items-center justify-center overflow-hidden rounded-md bg-(--logo-plate)", dim)} aria-hidden="true">
        {logo ? (
          <img src={logo} alt="" className="size-full object-contain p-xs" data-agent-logo={agent.id} />
        ) : (
          <span className="font-mono text-body font-semibold text-muted-foreground" data-agent-monogram={agent.id}>
            {monogram(agent.label)}
          </span>
        )}
      </span>
      <span className={cn("max-w-full truncate text-body font-semibold", switchable ? "text-foreground" : "text-muted-foreground")}>{agent.label}</span>
      <span className={cn("text-caption [&>span]:text-caption", status === "checking" ? "text-subtle-foreground" : "text-muted-foreground")} data-onboarding-status={status}>
        {line}
      </span>
    </>
  );
  // The tile grows past its size when the interface text is larger, so its mark keeps its size and no line is cut (B41).
  const base = "relative flex min-h-(--size-onboarding-tile) min-w-0 flex-col items-center justify-center gap-xs rounded-md border p-sm text-center";
  if (!switchable) {
    return (
      <div className={cn(base, "border-border")} data-onboarding-tile={`${agent.id}:unavailable`}>
        {body}
        {install ? (
          <a href={install} target="_blank" rel="noreferrer" className="rounded-xs text-caption font-medium text-foreground outline-none hover:underline focus-visible:ring-1 focus-visible:ring-ring" data-onboarding-install={agent.id}>
            {t("firstRun.tile.install")}
          </a>
        ) : null}
      </div>
    );
  }
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={agent.label}
      disabled={disabled}
      onClick={onToggle}
      className={cn(base, "cursor-pointer outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-default", on ? "border-primary bg-card" : "border-border bg-transparent hover:bg-accent")}
      data-onboarding-tile={`${agent.id}:${on ? "on" : "off"}`}
    >
      <span className={cn("absolute right-xs top-xs flex size-(--size-checkbox) items-center justify-center rounded-full border", on ? "border-primary bg-primary text-primary-foreground" : "border-border")} aria-hidden="true">
        {on ? <CheckIcon className="size-(--size-icon-sm)" /> : null}
      </span>
      {body}
    </button>
  );
}

function NotificationsStep({ actions, reopened, primary, onBack, answered, onAnswered }: StepProps & { onBack: () => void; answered: boolean; onAnswered: () => void }) {
  const { t } = useInterfaceTranslation();
  const control = useNotificationControl(actions, "wizard", usePermission());
  const state = control?.state ?? "off";
  const enabled = control?.enabled ?? false;
  // On a Mac that already allows Hide the step opens on, and Continue is what turns the switch on (B16).
  const allowedOff = !enabled && state === "on" && !answered;
  // Where nothing can be shown the step only says so and goes on (B47); until the switch is on it offers Turn on and Not now.
  const goesOn = answered || (enabled && state !== "asking") || allowedOff || state === "no_service";
  // Continue on a Mac that already allows Hide turns the switch on (B16); Not now leaves it off,
  // even after Turn on was pressed while macOS still asks (B21).
  const leave = (notifications: boolean | undefined) => {
    onAnswered();
    if (reopened) {
      if (notifications !== undefined) actions.setDesktopNotifications(notifications, notifications ? undefined : false);
      useSetupWalk.getState().go("project");
    } else actions.advanceFirstRun("notifications", notifications);
  };
  const next = () => leave(allowedOff ? true : undefined);
  // Enter presses the step's main button (B40): Continue, or Turn on while it can be pressed, a blocked Mac included.
  primary.current = goesOn ? next : control && state !== "asking" ? control.turnOn : null;
  const system = machineSystem(null);
  return (
    <>
      <DialogHeader>
        <DialogTitle>{t("firstRun.notifications.title")}</DialogTitle>
        <DialogDescription>
          {t(system === "macOS" ? "firstRun.notifications.descriptionMac" : system === "Windows" ? "firstRun.notifications.descriptionWindows" : "firstRun.notifications.description")}
        </DialogDescription>
      </DialogHeader>
      {control && state !== "off" ? (
        <DialogBody>
          <NotificationLine control={control} />
        </DialogBody>
      ) : null}
      <DialogFooter className={cn(!control || state === "off" ? "pt-md" : null)}>
        <Button variant="ghost" onClick={onBack} data-first-run-back="true">{t("firstRun.back")}</Button>
        <button type="button" onClick={() => useUiStore.getState().openSettings("mobile")} className="mr-auto rounded-xs text-body font-medium text-foreground outline-none hover:underline focus-visible:ring-1 focus-visible:ring-ring" data-first-run-phone="true">
          {t("firstRun.notifications.phone")}
        </button>
        {goesOn ? (
          // A blocked Mac still goes on, without the weight of the main button.
          <MainButton variant={state === "blocked" ? "secondary" : "default"} onClick={next} data-first-run-next="notifications">
            {t("firstRun.agents.continue")}
          </MainButton>
        ) : (
          <>
            {/* Not now leaves the switch off (B21); Turn on waits while macOS asks (B14). */}
            <Button variant="ghost" onClick={() => leave(false)} data-first-run-next="notifications">
              {t("firstRun.notifications.notNow")}
            </Button>
            {control ? (
              <Button onClick={control.turnOn} disabled={state === "asking"} data-notification-turn-on="true">
                {t("firstRun.notifications.turnOn")}
              </Button>
            ) : null}
          </>
        )}
      </DialogFooter>
    </>
  );
}

/**
 * The last step: how to start. Each way is its own button, with no choice to
 * confirm: practice first, which ends the wizard and opens the
 * practice window on chapter 0 over the empty Home; Add a project, the views
 * of Add project as they were, whose Back returns here; or Skip for now. Which
 * of the two is on screen is the project draft's view (`""` here), so a reload
 * reopens it.
 */
function StartStep({ actions, reopened, onBack }: { actions: Actions; reopened: boolean; onBack: () => void }) {
  const { t } = useInterfaceTranslation();
  const registrations = useShellStore((s) => s.rest?.ui_state?.workspace_registrations ?? NO_REGISTRATIONS);
  // The projects that were there when the step opened; the first one after them is the operator's (B23).
  const [known] = useState(() => new Set(registrations.map((row) => row.id)));
  // Only on the host the step adds to: a device's own registrations arriving meanwhile are not the operator's pick.
  const [host, setHost] = useState<string | null>(null);
  const added = registrations.find((row) => !row.home && !known.has(row.id) && row.device_id === host) ?? null;
  const [adding, setAdding] = useState(() => !!useShellStore.getState().rest?.ui_state?.first_run_choices?.project_form?.view);
  const finish = () => {
    if (reopened) useSetupWalk.getState().close();
    else actions.advanceFirstRun("project");
  };
  const showAdding = (next: boolean) => {
    setAdding(next);
    const form = useShellStore.getState().rest?.ui_state?.first_run_choices?.project_form;
    actions.saveFirstRunDraft({ project_form: { ...form, view: next ? "add" : "" } });
  };
  const practice = () => {
    finish();
    openPractice(FIRST_CHAPTER);
  };
  useEffect(() => {
    if (!added) return;
    finish();
    // The start panel opens once, on this project, with the first agent D-15 picks (B23, B24); with no agent on it does not open.
    const rest = useShellStore.getState().rest;
    const device = rest?.navigator?.devices?.find((device) => device.id === added.device_id);
    const agents = device?.kit?.agents ?? [];
    const kind = firstStartAgent(supportedAgents(agents), device?.kind === "remote" ? undefined : rest?.status?.background_ai?.providers);
    if (kind) useStartPanel.getState().open(false, { projectId: added.id, kind });
    // `finish` reads only the store; the project appearing is the one trigger.
  }, [added?.id]);
  const skip = (
    <Button variant="ghost" onClick={finish} data-first-run-skip="true">
      {t("firstRun.project.skip")}
    </Button>
  );
  if (adding) {
    return (
      <>
        <AddProjectViews actions={actions} title={(translate) => translate("firstRun.project.title")} onHost={setHost} draft={{ device: useShellStore.getState().rest?.ui_state?.first_run_choices?.project_device, path: useShellStore.getState().rest?.ui_state?.first_run_choices?.project_path, form: useShellStore.getState().rest?.ui_state?.first_run_choices?.project_form }} onDraft={actions.saveFirstRunDraft} />
        <DialogFooter>
          <Button variant="ghost" onClick={() => showAdding(false)} data-first-run-back="true">{t("firstRun.back")}</Button>
          {skip}
        </DialogFooter>
      </>
    );
  }
  return (
    <>
      <DialogHeader>
        <DialogTitle>{t("firstRun.start.title")}</DialogTitle>
        <DialogDescription>{t("firstRun.start.description")}</DialogDescription>
      </DialogHeader>
      <DialogBody className="flex flex-col gap-sm" data-first-run-start="true">
        {/* The practice coach's own card, so it reads as practice and stays legible in Light and Dark. */}
        <StartWay
          icon={<GraduationCapIcon />}
          title={t("firstRun.start.practice")}
          line={t("firstRun.start.practiceNote")}
          className="border-(--practice-accent) bg-(--practice-card) text-(--practice-text) hover:border-(--practice-accent-strong)"
          lineClassName="text-(--practice-mute)"
          iconClassName="border-(--practice-line) bg-(--practice-keycap) text-(--practice-ink)"
          onClick={practice}
          data-first-run-way="practice"
        />
        <StartWay
          icon={<FolderPlusIcon />}
          title={t("firstRun.start.project")}
          line={t("firstRun.start.projectNote")}
          className="border-border bg-secondary text-foreground hover:bg-accent"
          lineClassName="text-muted-foreground"
          iconClassName="border-border bg-background text-muted-foreground"
          onClick={() => showAdding(true)}
          data-first-run-way="project"
        />
      </DialogBody>
      <DialogFooter>
        <Button variant="ghost" onClick={onBack} data-first-run-back="true">{t("firstRun.back")}</Button>
        <span className="flex-1" />
        {skip}
      </DialogFooter>
    </>
  );
}

/** One way to start: a whole-width button with its mark, its name and one line under it. */
function StartWay({ icon, title, line, className, lineClassName, iconClassName, ...props }: ComponentProps<"button"> & { icon: ReactNode; title: string; line: string; lineClassName: string; iconClassName: string }) {
  return (
    <button
      type="button"
      className={cn("flex w-full cursor-pointer items-center gap-md rounded-lg border px-md py-sm text-left outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring", className)}
      {...props}
    >
      <span className={cn("flex size-(--size-icon-button-standard) shrink-0 items-center justify-center rounded-md border [&_svg]:size-(--size-icon)", iconClassName)} aria-hidden="true">
        {icon}
      </span>
      <span className="flex min-w-0 flex-1 flex-col gap-xxs">
        <span className="text-body font-semibold">{title}</span>
        <span className={cn("text-caption", lineClassName)}>{line}</span>
      </span>
      <ChevronRightIcon aria-hidden="true" className={cn("size-(--size-icon) shrink-0", lineClassName)} />
    </button>
  );
}

/** A chosen agent that did not go in, with why when this build can word it. */
function failureLine(failure: FailedAgent, t: TFunction<"translation">): string {
  const reason = kitWhyText(failure.why, t);
  return reason ? t("firstRun.agents.failed", { agent: failure.label, reason }) : t("firstRun.agents.failedNoReason", { agent: failure.label });
}
