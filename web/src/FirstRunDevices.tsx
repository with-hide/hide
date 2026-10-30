// The first-run Devices step: the machines this computer already knows, the
// Hosts of its ssh config (Add device's list) and the profiles Herdr saved,
// one row per machine with a chip for where it came from. Reading them
// contacts no machine; only the explicit Connect press admits the checked
// rows to the existing consent and connection owner, Add device's for a Host.
// What ssh asks after the press is answered on the row with the device row's
// own sign-in part, and a failure reads as the device row's three states.
// Next and Not now are never held by a question, and the step going away
// ends the sign-ins its Connect started (the core knows which).
// On Windows this PC's default WSL distribution is a row too, checked until
// the operator unchecks it (PRD wsl-device B1); its press installs Hide there
// and brings its tile to the front (B2), and one on WSL 1 shows the command
// that switches it instead of a checkbox it could use (B29).
import { useEffect, useRef, useState } from "react";
import type { Actions } from "./actions";
import { Note, Status } from "./components/settings-rows";
import { Badge } from "./components/ui/badge";
import { Button } from "./components/ui/button";
import { Checkbox } from "./components/ui/checkbox";
import { DialogBody, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "./components/ui/dialog";
import { useInterfaceTranslation } from "./i18n/client";
import type { MessageKey } from "./i18n/catalogs";
import type { HerdrProfile } from "./snapshot";
import { DeviceInstallTerms } from "./settings/AddDevice";
import { DeviceLink, deviceLinkShown } from "./settings/DeviceRow";
import { canRetryDevice, retrySignsIn } from "./settings";
import { useShellStore } from "./store";
import { remoteStateText } from "./coreError";
import { profileRefusalText } from "./kitWhy";
import { hostProblemReason, wslConvertCommand } from "./sshHosts";

export function FirstRunDevices({ actions, primary, onBack, onNext }: {
  actions: Actions;
  primary: { current: (() => void) | null };
  onBack: () => void;
  onNext: () => void;
}) {
  const { t } = useInterfaceTranslation();
  const rest = useShellStore((s) => s.rest);
  const profiles = rest?.status?.herdr_profiles;
  const [picked, setPicked] = useState<ReadonlySet<string> | null>(() => {
    const draft = rest?.ui_state?.first_run_choices?.devices;
    return draft ? new Set(draft) : null;
  });
  const [sent, setSent] = useState<ReadonlySet<string>>(new Set());
  const revision = useShellStore((s) => s.revision);
  const [pressedAt, setPressedAt] = useState<number | null>(null);
  // The step going away, by any of its buttons or Help's walk, ends the sign-ins its Connect started.
  const pressed = useRef(false);
  const latest = useRef(actions);
  latest.current = actions;
  useEffect(() => () => {
    if (pressed.current) latest.current.withdrawFirstRunSignIns();
  }, []);
  const rows = profiles?.profiles ?? [];
  // Until a row is checked or unchecked, only the default WSL distribution is (B1).
  const selected = picked ?? new Set(rows.filter((profile) => profile.wsl_distro && !profile.problem && !profile.added_as).map((profile) => profile.id));
  const wsl = rows.some((profile) => profile.wsl_distro);
  const reading = !profiles || profiles.state === "idle" || profiles.state === "loading";
  const failed = profiles?.state === "failed";
  const deviceOf = (profile: HerdrProfile) => rest?.navigator?.devices?.find((row) => row.id === profile.device_id);
  const rowState = (profile: HerdrProfile): RowState => {
    if (!profile.enabled) return "disabled";
    if (profile.problem) return "unavailable";
    const deviceId = profile.device_id ?? profile.added_as;
    const remote = rest?.status?.remote?.find((row) => row.target_id === deviceId);
    const kit = rest?.navigator?.devices?.find((row) => row.id === deviceId)?.kit;
    // ssh asks the operator now; nothing else about the row matters until they answer.
    if (deviceOf(profile)?.sign_in) return "signing_in";
    if (sent.has(profile.id) && pressedAt === revision) return "connecting";
    if (sent.has(profile.id) || ["connecting", "connected", "failed"].includes(profile.connect_state)) {
      if (kit?.busy) return "installing";
      if (kit?.unavailable || kit?.refused || kit?.components.some((part) => part.state === "failed") || kit?.agents.some((agent) => agent.enabled && [agent.skill, agent.hook, agent.herdr].some((piece) => piece?.state === "failed"))) return "failed";
      if (remote?.state === "connected" && kit && !kit.busy) return "connected";
      if (remote?.state === "connecting") return "connecting";
      if (profile.connect_state === "failed" || (remote && !["connected", "not_connected"].includes(remote.state)) || (remote?.state === "not_connected" && remote.message)) return "failed";
      if (pressedAt === revision || profile.connect_state === "connecting" || remote?.state === "connecting" || !remote) return "connecting";
    }
    if (profile.added_as) return "added";
    return profile.connect_state === "failed" ? "failed" : "saved";
  };
  const states = new Map(rows.map((profile) => [profile.id, rowState(profile)] as const));
  const busy = [...states.values()].some((state) => state === "connecting" || state === "installing");
  // A failed row is tried again by its own action only (Sign in, Retry, none for a changed server key).
  const targets = rows.filter((profile) => selected.has(profile.id) && states.get(profile.id) === "saved" && profile.enabled && !profile.problem);
  const connect = (only?: HerdrProfile) => {
    if (busy) return;
    const chosen = only ? [only] : targets;
    if (chosen.length === 0) return;
    pressed.current = true;
    setPressedAt(revision);
    setSent((previous) => new Set([...previous, ...chosen.map((row) => row.id)]));
    for (const profile of chosen) actions.connectHerdrProfile(profile.id);
  };
  // A proven empty result skips this step, including Help's local walk.
  useEffect(() => { if (profiles?.state === "ready" && rows.length === 0 && !profiles.truncated) onNext(); }, [profiles?.state, rows.length]);
  // A waiting question never holds the way on: Next leaves it, and leaving ends it.
  const connectable = targets.length > 0 && !busy;
  primary.current = !reading && !failed ? connectable ? () => connect() : onNext : null;
  const toggle = (id: string) => {
    const next = new Set(selected);
    if (!next.delete(id)) next.add(id);
    setPicked(next);
    actions.saveFirstRunDraft({ devices: [...next] });
  };
  const shown = [...states.values()];
  return (
    <>
      <DialogHeader>
        <DialogTitle>{t("firstRun.devices.title")}</DialogTitle>
        <DialogDescription className="break-keep">{t(wsl ? "firstRun.devices.descriptionWsl" : "firstRun.devices.description")}</DialogDescription>
      </DialogHeader>
      <DialogBody className="flex flex-col gap-md" data-first-run-devices={reading ? "loading" : failed ? "failed" : rows.length === 0 ? "empty" : "ready"}>
        {reading ? <Status tone="pending">{t("firstRun.devices.loading")}</Status> : failed ? (
          <div role="status" className="flex flex-wrap items-center gap-sm">
            <Status tone="error">{t("firstRun.devices.readFailed")}</Status>
            <Button variant="secondary" onClick={actions.listHerdrProfiles} data-first-run-devices-retry="true">{t("common.retry")}</Button>
          </div>
        ) : rows.length === 0 ? <Note>{t("firstRun.devices.empty")}</Note> : (
          <div role="group" aria-label={t("firstRun.devices.selection")} className="divide-y divide-border rounded-md border border-border bg-card">
            {rows.map((profile) => {
              const state = states.get(profile.id)!;
              const device = deviceOf(profile);
              const remote = rest?.status?.remote?.find((row) => row.target_id === profile.device_id);
              // The device row's own part says what ssh asks, or why it stopped, with that row's one action.
              const link = device && deviceLinkShown(device) ? device : null;
              // A device's connection is worded from its state; the core's sentence beside it is the log's.
              const reason = profile.connect_refusal ? profileRefusalText(profile.connect_refusal, t) : (remote && remote.state !== "connected" && remote.state !== "not_connected" ? remoteStateText(remote.state, profile.label, t) : null);
              const retry = state === "failed" && (link?.problem == null || canRetryDevice(link, remote));
              const locked = busy || ["added", "connected", "disabled", "unavailable", "signing_in"].includes(state);
              const distro = profile.wsl_distro ?? null;
              const here = distro ? t("firstRun.devices.wslHere") : null;
              const quiet = state === "saved" || (distro !== null && state === "unavailable");
              const hostReason = distro === null && state === "unavailable" ? hostProblemReason(profile.problem) : null;
              return (
                <div key={profile.id} className="flex flex-wrap items-start gap-sm px-md py-sm" data-first-run-device={profile.id} data-device-state={state} data-first-run-wsl={distro ?? undefined}>
                  <Checkbox id={`herdr-profile-${profile.id}`} checked={selected.has(profile.id)} disabled={locked} onCheckedChange={() => toggle(profile.id)} aria-label={here ? `${profile.label} · ${here}` : profile.label} className="mt-xxs" />
                  <div className="flex min-w-0 flex-1 flex-col gap-xs">
                    <label htmlFor={`herdr-profile-${profile.id}`} className="flex min-w-0 flex-col gap-xxs">
                      <span className="break-words text-subhead font-medium text-foreground">{profile.label}</span>
                      {here ? <span className="break-keep text-caption text-muted-foreground">{here}</span>
                        : <span className="break-all font-mono text-caption text-muted-foreground">{profile.address ?? profile.target}</span>}
                    </label>
                    {here ? null : (
                      <span className="flex flex-wrap gap-xs" data-first-run-device-sources={profile.id}>
                        {profile.ssh_host ? <Badge variant="outline" data-first-run-device-source="ssh">{t("firstRun.devices.fromSsh")}</Badge> : null}
                        {profile.herdr ? <Badge variant="outline" data-first-run-device-source="herdr">{profile.session === "default" ? t("firstRun.devices.saved") : `${t("firstRun.devices.saved")} · ${t("firstRun.devices.session", { session: profile.session })}`}</Badge> : null}
                      </span>
                    )}
                    {link ? <DeviceLink device={link} actions={actions} /> : null}
                    {state === "failed" && !link ? <Status tone="error">{t("firstRun.devices.failed")}</Status> : null}
                    {state === "failed" && !link && reason ? <span className="break-words text-caption text-muted-foreground">{reason}</span> : null}
                    {state === "unavailable" && distro ? <WslProblem distro={distro} problem={profile.problem} /> : null}
                    {state === "unavailable" && !distro ? <Status tone="warn">{t(hostReason ?? "firstRun.devices.unavailable")}</Status> : null}
                  </div>
                  <span className="max-w-full text-caption" aria-live="polite">
                    {retry ? <Button size="sm" variant="secondary" disabled={busy} onClick={() => connect(profile)} data-first-run-device-retry={profile.id}>{retrySignsIn(link?.problem) ? t("devices.signIn.start") : t("common.retry")}</Button>
                      : quiet || state === "failed" || state === "unavailable" ? null
                      : <Status tone={state === "connected" ? "ok" : state === "connecting" || state === "installing" || state === "signing_in" ? "pending" : "muted"}>{t(DEVICE_STATE_LABEL[state])}</Status>}
                  </span>
                </div>
              );
            })}
          </div>
        )}
        {profiles?.truncated ? <Note>{t("firstRun.devices.truncated")}</Note> : null}
        {!reading && !failed && rows.length > 0 ? (
          <>
            {shown.includes("signing_in") ? <Note data-first-run-devices-note="leave">{t("firstRun.devices.leaveEnds")}</Note> : null}
            {shown.includes("failed") ? <Note data-first-run-devices-note="failed">{t("firstRun.devices.failedStays")}</Note> : null}
            <Note data-first-run-devices-note="not-listed">{t("firstRun.devices.notListed")}</Note>
            <details className="text-body"><summary className="cursor-pointer text-foreground">{t("devices.whatInstalls")}</summary><div className="pt-sm"><DeviceInstallTerms helperRoot={null} cliDir={null} /></div></details>
          </>
        ) : null}
      </DialogBody>
      <DialogFooter>
        <Button variant="ghost" onClick={onBack} data-first-run-back="true">{t("firstRun.back")}</Button>
        <Button variant="ghost" onClick={onNext} data-first-run-device-skip="true">{t("firstRun.devices.onlyHere")}</Button>
        {!reading && !failed ? <Button onClick={() => connectable ? connect() : onNext()} data-first-run-device-connect="true">{connectable ? t("firstRun.devices.connect", { count: targets.length }) : t("firstRun.agents.continue")}</Button> : null}
      </DialogFooter>
    </>
  );
}

/** Why this PC's WSL distribution cannot connect: its setup is unfinished, or it runs on WSL 1, with the command that switches it (B29). */
function WslProblem({ distro, problem }: { distro: string; problem: string | null }) {
  const { t } = useInterfaceTranslation();
  if (problem === "wsl1") {
    return (
      <span className="flex flex-col gap-xxs text-caption text-muted-foreground" data-first-run-wsl-problem="wsl1">
        <span className="break-keep">{t("devices.wsl.version1")}</span>
        <span className="break-words font-mono text-subtle-foreground" data-first-run-wsl-command={distro}>{wslConvertCommand(distro)}</span>
      </span>
    );
  }
  return <span className="break-keep text-caption text-muted-foreground" data-first-run-wsl-problem={problem ?? undefined}>{t(problem === "setup_unfinished" ? "devices.wsl.setupUnfinished" : "firstRun.devices.unavailable")}</span>;
}

const DEVICE_STATE_LABEL = {
  saved: "firstRun.devices.saved", disabled: "firstRun.devices.disabled", added: "firstRun.devices.added", connected: "firstRun.devices.connected", connecting: "firstRun.devices.connecting", installing: "firstRun.devices.installing", failed: "firstRun.devices.failed", unavailable: "firstRun.devices.unavailable", signing_in: "firstRun.devices.signingIn",
} satisfies Record<string, MessageKey>;

type RowState = keyof typeof DEVICE_STATE_LABEL;
