// Saved configuration is not reachability. Only the explicit Connect press
// admits selected profiles to the existing consent and connection owner.
// On Windows this PC's default WSL distribution is a row too, checked until
// the operator unchecks it (PRD wsl-device B1); its press installs Hide there
// and brings its tile to the front (B2), and one on WSL 1 shows the command
// that switches it instead of a checkbox it could use (B29).
import { useEffect, useState } from "react";
import type { Actions } from "./actions";
import { Note, Status } from "./components/settings-rows";
import { Button } from "./components/ui/button";
import { Checkbox } from "./components/ui/checkbox";
import { DialogBody, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "./components/ui/dialog";
import { useInterfaceTranslation } from "./i18n/client";
import type { MessageKey } from "./i18n/catalogs";
import type { HerdrProfile } from "./snapshot";
import { DeviceInstallTerms } from "./settings/AddDevice";
import { useShellStore } from "./store";
import { remoteStateText } from "./coreError";
import { profileRefusalText } from "./kitWhy";
import { wslConvertCommand } from "./sshHosts";

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
  const rows = profiles?.profiles ?? [];
  // Until a row is checked or unchecked, the default WSL distribution is (B1).
  const selected = picked ?? new Set(rows.filter((profile) => profile.wsl_distro && !profile.problem && !profile.added_as).map((profile) => profile.id));
  const wsl = rows.some((profile) => profile.wsl_distro);
  const reading = !profiles || profiles.state === "idle" || profiles.state === "loading";
  const failed = profiles?.state === "failed";
  const rowState = (profile: HerdrProfile) => {
    if (!profile.enabled) return "disabled";
    if (profile.problem) return "unavailable";
    const deviceId = profile.device_id ?? profile.added_as;
    const remote = rest?.status?.remote?.find((row) => row.target_id === deviceId);
    const kit = rest?.navigator?.devices?.find((row) => row.id === deviceId)?.kit;
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
  const busy = rows.some((profile) => ["connecting", "installing"].includes(rowState(profile)));
  const targets = rows.filter((profile) => selected.has(profile.id) && ["saved", "failed"].includes(rowState(profile)) && profile.enabled && !profile.problem);
  const connect = (only?: HerdrProfile) => {
    if (busy) return;
    const chosen = only ? [only] : targets;
    if (chosen.length === 0) return;
    setPressedAt(revision);
    setSent((previous) => new Set([...previous, ...chosen.map((row) => row.id)]));
    for (const profile of chosen) actions.connectHerdrProfile(profile.id);
  };
  // A proven empty result skips this step, including Help's local walk.
  useEffect(() => { if (profiles?.state === "ready" && rows.length === 0) onNext(); }, [profiles?.state, rows.length]);
  primary.current = !reading && !failed && !busy ? targets.length > 0 ? () => connect() : onNext : null;
  const toggle = (id: string) => {
    const next = new Set(selected);
    if (!next.delete(id)) next.add(id);
    setPicked(next);
    actions.saveFirstRunDraft({ devices: [...next] });
  };
  return (
    <>
      <DialogHeader>
        <DialogTitle>{t(wsl ? "firstRun.devices.titleWsl" : "firstRun.devices.title")}</DialogTitle>
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
              const state = rowState(profile);
              // A device's connection is worded from its state; the core's sentence beside it is the log's.
              const remote = rest?.status?.remote?.find((row) => row.target_id === profile.device_id);
              const reason = profile.connect_refusal ? profileRefusalText(profile.connect_refusal, t) : (remote && remote.state !== "connected" && remote.state !== "not_connected" ? remoteStateText(remote.state, profile.label, t) : null);
              const locked = busy || ["added", "connected", "disabled", "unavailable"].includes(state);
              const distro = profile.wsl_distro ?? null;
              // A WSL row is named where it is, and only Herdr's own rows read "Saved in Herdr".
              const here = distro ? t("firstRun.devices.wslHere") : null;
              const quiet = distro !== null && (state === "saved" || state === "unavailable");
              return (
                <div key={profile.id} className="flex flex-wrap items-start gap-sm px-md py-sm" data-first-run-device={profile.id} data-device-state={state} data-first-run-wsl={distro ?? undefined}>
                  <Checkbox id={`herdr-profile-${profile.id}`} checked={selected.has(profile.id)} disabled={locked} onCheckedChange={() => toggle(profile.id)} aria-label={here ? `${profile.label} · ${here}` : profile.label} className="mt-xxs" />
                  <label htmlFor={`herdr-profile-${profile.id}`} className="flex min-w-0 flex-1 flex-col gap-xs">
                    <span className="break-words text-subhead font-medium text-foreground">{profile.label}</span>
                    {here ? <span className="break-keep text-caption text-muted-foreground">{here}</span>
                      : <span className="break-all font-mono text-caption text-muted-foreground">{profile.target} · {t("firstRun.devices.session", { session: profile.session })}</span>}
                    {state === "failed" ? <Status tone="error">{t("firstRun.devices.failed")}</Status> : null}
                    {state === "failed" && reason ? <span className="break-words text-caption text-muted-foreground">{reason}</span> : null}
                    {state === "unavailable" && distro ? <WslProblem distro={distro} problem={profile.problem} /> : null}
                    {state === "unavailable" && !distro ? <Status tone="warn">{t("firstRun.devices.unavailable")}</Status> : null}
                  </label>
                  <span className="max-w-full text-caption" aria-live="polite">
                    {state === "failed" ? <Button size="sm" variant="secondary" disabled={busy} onClick={() => connect(profile)} data-first-run-device-retry={profile.id}>{t("common.retry")}</Button>
                      : quiet ? null
                      : <Status tone={state === "connected" ? "ok" : state === "connecting" || state === "installing" ? "pending" : "muted"}>{t(DEVICE_STATE_LABEL[state])}</Status>}
                  </span>
                </div>
              );
            })}
          </div>
        )}
        {profiles?.truncated ? <Note>{t("firstRun.devices.truncated")}</Note> : null}
        {!reading && !failed && rows.length > 0 ? (
          <>
            <p className="text-caption text-muted-foreground">{t("firstRun.devices.savedHint")}</p>
            <p className="text-body text-subtle-foreground">{t("firstRun.devices.consent")}</p>
            <details className="text-body"><summary className="cursor-pointer text-foreground">{t("devices.whatInstalls")}</summary><div className="pt-sm"><DeviceInstallTerms helperRoot={null} cliDir={null} /></div></details>
          </>
        ) : null}
      </DialogBody>
      <DialogFooter>
        <Button variant="ghost" onClick={onBack} data-first-run-back="true">{t("firstRun.back")}</Button>
        <Button variant="ghost" onClick={onNext} disabled={busy} data-first-run-device-skip="true">{t("firstRun.notifications.notNow")}</Button>
        {!reading && !failed ? <Button disabled={busy} onClick={() => targets.length > 0 ? connect() : onNext()} data-first-run-device-connect="true">{busy ? t("firstRun.devices.connecting") : targets.length > 0 ? t("firstRun.devices.connect", { count: targets.length }) : t("firstRun.agents.continue")}</Button> : null}
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
  saved: "firstRun.devices.saved", disabled: "firstRun.devices.disabled", added: "firstRun.devices.added", connected: "firstRun.devices.connected", connecting: "firstRun.devices.connecting", installing: "firstRun.devices.installing", failed: "firstRun.devices.failed", unavailable: "firstRun.devices.unavailable",
} satisfies Record<string, MessageKey>;
