// The Add device dialog's body (PRD settings-cleanup D-19, B50 to B53): the
// account's ssh config Hosts to choose from, each with the address ssh
// resolves it to; a chosen Host fills the name, and Add stores only that name
// and the alias. Username, port and key stay in ssh's own config, so there is
// no input for them. On Windows this PC's WSL distributions are listed first,
// under the same consent (PRD wsl-device B4); a distribution's device takes
// the name the core gives it (D-20). The list is hided's
// (`status.ssh_hosts`), asked for when the dialog opens.
//
// A chosen Host is asked, over an ssh that asks nobody, whether a Hide of
// its own runs there (PRD device-connect-iroh B7), and a chosen distribution
// through wsl.exe (D-27), which starts it when it is off (B43). One that
// does is connected to as a Hide, which installs nothing there, and the
// form says so before the press; one whose Hide is too old or not running
// says what to do on that machine; any other is added and used over SSH
// or WSL, and nothing switches it but the operator's own choice.

import { useEffect, useId, useState } from "react";
import type { Actions } from "../actions";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { RadioGroup, RadioGroupItem } from "../components/ui/radio-group";
import { Disclosure, Group, Note, Row, Status } from "../components/settings-rows";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import type { Device } from "../snapshot";
import { chosenDistro, defaultDeviceName, hostChoices, pickableAliases, wslChoices, type HostChoice, type WslChoice } from "../sshHosts";
import { useShellStore } from "../store";
import { coreErrorText } from "../coreError";
import { isPairLink, refusalKey, sshLookLine, type SshLook } from "../pairing";
import { useRefusalSince } from "../errorSince";

const HOST_EXAMPLE = "Host studio\n  HostName studio.local\n  User you";

/**
 * Add a device by the link another Hide shared (PRD device-connect-iroh
 * B2): the field takes the whole link, and Connect starts the pairing, whose
 * words and outcome the pairing window shows wherever the operator is.
 */
export function AddByLink({ actions, onStarted }: { actions: Actions; onStarted: () => void }) {
  const { t } = useInterfaceTranslation();
  const [link, setLink] = useState("");
  const [sentAt, setSentAt] = useState<string | null>(null);
  const result = useShellStore((s) => (s.pairingResult?.verb === "join" && s.pairingResult.request_id === sentAt ? s.pairingResult : null));
  const refused = result && !result.ok ? (refusalKey(result.reason) ?? "pairing.failed.ended") : null;
  // The pairing window takes over once the daemon took the link.
  useEffect(() => {
    if (result?.ok) onStarted();
  }, [result, onStarted]);
  const id = useId();
  const submit = () => {
    if (!isPairLink(link)) return;
    const request = `join-${Date.now()}`;
    setSentAt(request);
    actions.pairing("join", { link: link.trim(), request_id: request });
  };
  return (
    <section className="space-y-sm" data-add-by-link="true">
      <p id={id} className="break-keep text-body text-muted-foreground">{t("pairing.add.linkDescription")}</p>
      <div className="flex items-center gap-sm">
        <Input
          aria-labelledby={id}
          value={link}
          placeholder={t("pairing.add.linkPlaceholder")}
          spellCheck={false}
          autoComplete="off"
          className="font-mono"
          data-add-link-input="true"
          onChange={(event) => setLink(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") submit();
          }}
        />
        <Button variant="secondary" disabled={!isPairLink(link)} data-add-link-go="true" onClick={submit}>{t("pairing.add.connect")}</Button>
      </div>
      {refused ? <Note tone="warn" data-add-link-refused={result?.reason ?? ""}>{t(refused)}</Note> : null}
    </section>
  );
}

export function AddDevice({ actions, devices, helperRoot, cliDir, onPaired }: { actions: Actions; devices: Device[]; helperRoot: string | null; cliDir: string | null; onPaired: () => void }) {
  const { t } = useInterfaceTranslation();
  const hosts = useShellStore((s) => s.rest?.status?.ssh_hosts);
  const answered = useShellStore((s) => s.pairingResult);
  // What the chosen Host's own Hide answered (nothing, when the look was
  // refused), and the pairing a press started.
  const [look, setLook] = useState<{ alias: string; request: string; answered: boolean; looked: SshLook | null } | null>(null);
  const [pairing, setPairing] = useState<{ request: string; refused: string | null } | null>(null);
  // The Host whose own Hide the operator chose not to connect to: it is added as an SSH device, as a Host with no Hide is.
  const [overSsh, setOverSsh] = useState<string | null>(null);
  useEffect(() => {
    if (!answered?.request_id) return;
    if ((answered.verb === "look_ssh" || answered.verb === "look_wsl") && answered.request_id === look?.request && !look.answered) {
      setLook({ ...look, answered: true, looked: answered.ok ? (answered.looked ?? null) : null });
    }
    if ((answered.verb === "pair_ssh" || answered.verb === "pair_wsl") && answered.request_id === pairing?.request && pairing.refused === null) {
      if (answered.ok) onPaired();
      else setPairing({ ...pairing, refused: answered.reason ?? "" });
    }
  }, [answered, look, pairing, onPaired]);
  const [alias, setAlias] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [nameEdited, setNameEdited] = useState(false);
  // The core gives a new device its id (its machine's own, once it first
  // connects), so the dialog waits for the alias or distribution it added.
  const [submitted, setSubmitted] = useState<{ alias: string | null; distro: string | null; at: number } | null>(null);
  const error = useRefusalSince(submitted?.at ?? null, ["device."]);
  const choices = hostChoices(hosts?.hosts ?? []);
  const distributions = wslChoices(hosts?.wsl ?? []);
  const pickable = [...distributions.filter((choice) => choice.kind === "available").map((choice) => choice.value), ...pickableAliases(choices)];
  // The dialog opening is the one request: hided reads the config then, not on a timer.
  useEffect(() => {
    actions.listSshHosts();
  }, [actions]);
  // A Host that stopped being pickable while chosen (the list was read again) is no longer chosen.
  const chosen = alias !== null && pickable.includes(alias) ? alias : null;
  const distro = chosen === null ? null : chosenDistro(chosen);
  const distribution = distributions.find((choice) => choice.value === chosen) ?? null;
  const added = submitted ? devices.some((device) => (submitted.distro !== null ? device.wsl_distro === submitted.distro : device.ssh_alias === submitted.alias)) : false;
  const looked = look !== null && look.alias === chosen ? look.looked : null;
  const lookLine = sshLookLine(looked);
  // The chosen Host runs its own Hide: the press connects, and installs
  // nothing, unless the operator asked to add it over SSH all the same,
  // which leaves that Hide as it is.
  const hasHide = looked?.state === "ready";
  const connects = hasHide && overSsh !== chosen;
  const connecting = pairing !== null && pairing.refused === null;
  const pending = (submitted !== null && !added && error === null) || connecting;
  // Until the Host answered, a press would add a machine that runs its own
  // Hide as a helper device: the press waits for the answer.
  const looking = look !== null && look.alias === chosen && !look.answered;
  const blocked = pending || looking || chosen === null || (distro === null && !connects && !name.trim());
  const choose = (next: string) => {
    setAlias(next);
    setPairing(null);
    setOverSsh(null);
    const picked = chosenDistro(next);
    // The name follows the Host until the person writes their own; a
    // distribution's is the core's.
    if (picked === null && !nameEdited) setName(defaultDeviceName(next));
    const request = `look-${Date.now()}`;
    setLook({ alias: next, request, answered: false, looked: null });
    if (picked === null) actions.pairing("look_ssh", { alias: next, request_id: request });
    else actions.pairing("look_wsl", { distro: picked, request_id: request });
  };
  // Adding is where the whole kit is agreed to, once (PRD device-parity
  // D-12): what Hide installs is under Advanced and the description above
  // says it, and there is one way to add.
  const submit = () => {
    if (blocked || chosen === null) return;
    if (connects) {
      const request = `pair-${Date.now()}`;
      setPairing({ request, refused: null });
      if (distro !== null) actions.pairing("pair_wsl", { distro, request_id: request });
      else actions.pairing("pair_ssh", { alias: chosen, request_id: request });
      return;
    }
    // A device added over SSH or wsl.exe is used over that login, and
    // Hide moves it to its machine's own connection by itself when that
    // machine can take it (operator decision 2026-11-24): the Add asks
    // nothing more and never ends without the device.
    if (distro !== null) {
      setSubmitted({ alias: null, distro, at: Date.now() });
      actions.registerWslDevice(distro);
    } else {
      setSubmitted({ alias: chosen, distro: null, at: Date.now() });
      actions.registerDevice(name.trim(), chosen, { hostConsent: true });
    }
  };
  // Until hided has answered there is nothing to choose; an answer with no Host says what to add (B52).
  const reading = hosts === undefined || ((hosts.state === "idle" || hosts.state === "loading") && hosts.hosts.length === 0 && distributions.length === 0);
  if (reading) return <Note tone="pending" data-ssh-hosts-state="loading">{t("devices.hostsLoading")}</Note>;
  const unreadable = hosts.wsl_problem === "unreadable" ? <Note tone="warn" data-wsl-problem="unreadable">{t("devices.wsl.unreadable")}</Note> : null;
  if (choices.length === 0 && distributions.length === 0) return <>{unreadable}<NoHosts /></>;
  return (
    <div className="space-y-md" data-add-device-form="true">
      <p className="break-keep text-body text-muted-foreground">{t(distributions.length > 0 ? "devices.addDescriptionWsl" : "devices.addDescription")}</p>
      <RadioGroup
        value={chosen ?? ""}
        onValueChange={choose}
        aria-label={t("devices.hostsAria")}
        className="gap-none divide-y divide-border rounded-md border border-border bg-card"
        data-ssh-hosts-state={hosts.state}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            submit();
          }
        }}
      >
        {distributions.map((choice) => (
          <WslRow key={choice.value} choice={choice} disabled={pending} />
        ))}
        {choices.map((choice) => (
          <HostRow key={choice.alias} choice={choice} disabled={pending} />
        ))}
      </RadioGroup>
      {unreadable}
      {hosts.truncated ? <Note>{t("devices.hostsTruncated", { count: hosts.hosts.length })}</Note> : null}
      <Note data-ssh-hosts-hint="true">{t("devices.notListed")}</Note>
      {looking ? <Note tone="pending" data-ssh-look="looking">{t("pairing.ssh.looking")}</Note> : null}
      {lookLine ? <Status tone={lookLine.tone} data-ssh-look={looked?.state}>{t(lookLine.key)}</Status> : null}
      {pairing?.refused != null ? <Note tone="warn" data-ssh-pair-refused={pairing.refused}>{t(refusalKey(pairing.refused) ?? "pairing.failed.ended")}</Note> : null}
      {connects ? null : (
      <Group>
        <Row label={t("devices.name")} detail={error ? <Note tone="error" data-add-device-error="true" data-core-error={error.kind}>{coreErrorText(error, t)}</Note> : null}>
          <Input
            value={distribution?.deviceName ?? name}
            disabled={pending || distribution !== null}
            aria-label={t("devices.nameAria")}
            className="w-(--size-settings-control-w)"
            onChange={(event) => {
              setNameEdited(true);
              setName(event.target.value);
            }}
            data-device-label="true"
          />
        </Row>
        <Disclosure title={t("devices.advanced")} data-add-device-advanced="true">
              <Row label={<span className="text-subhead text-subtle-foreground">{t("devices.whatInstalls")}</span>} detail={<DeviceInstallTerms helperRoot={helperRoot} cliDir={cliDir} />} />
        </Disclosure>
      </Group>
      )}
      <div className="flex items-center justify-end gap-md">
        {connects && !connecting ? (
          <Button variant="text" onClick={() => setOverSsh(chosen)} data-add-device-over-ssh="true">
            {t("pairing.add.overSsh")}
          </Button>
        ) : null}
        <Button disabled={blocked} onClick={submit} data-add-device={connects ? "connect" : "true"}>
          {connects ? (connecting ? t("pairing.ssh.connecting") : t("pairing.add.connect")) : pending ? t("devices.adding") : t("common.add")}
        </Button>
      </div>
    </div>
  );
}

/** One Host: chosen with a radio, or dimmed with why it cannot be (B51, B52). */
function HostRow({ choice, disabled }: { choice: HostChoice; disabled: boolean }) {
  const { t } = useInterfaceTranslation();
  const note = useId();
  const unavailable = choice.kind !== "available";
  const reason = choice.kind === "added" ? t("devices.hostAdded", { name: choice.name }) : choice.kind === "unresolved" ? t(choice.reason) : null;
  const address = choice.kind === "unresolved" ? null : choice.address;
  return (
    <label className={cn("flex items-start gap-md px-md py-sm", unavailable ? "cursor-not-allowed text-muted-foreground" : "cursor-pointer")} data-ssh-host={choice.alias} data-ssh-host-state={choice.kind}>
      <RadioGroupItem value={choice.alias} disabled={unavailable || disabled} aria-label={choice.alias} aria-describedby={reason ? note : undefined} className="mt-xxs" />
      <span className="flex min-w-0 flex-1 flex-col gap-xxs">
        <span className={cn("break-all text-subhead", unavailable ? "text-muted-foreground" : "text-foreground")}>{choice.alias}</span>
        {address ? <span className="break-all font-mono text-body text-muted-foreground">{address}</span> : null}
      </span>
      {reason ? (
        <span id={note} className="max-w-1/2 shrink-0 break-words break-keep text-right text-body text-muted-foreground" data-ssh-host-note={choice.alias}>
          {reason}
        </span>
      ) : null}
    </label>
  );
}

/** One WSL distribution of this PC: chosen like a Host, or dimmed with why it cannot be (PRD wsl-device B4, B28). */
function WslRow({ choice, disabled }: { choice: WslChoice; disabled: boolean }) {
  const { t } = useInterfaceTranslation();
  const note = useId();
  const unavailable = choice.kind !== "available";
  const reason = choice.kind === "added" ? t("devices.hostAdded", { name: choice.name }) : choice.kind === "unavailable" ? t(choice.reason) : null;
  const command = choice.kind === "unavailable" ? choice.command : null;
  return (
    <label className={cn("flex items-start gap-md px-md py-sm", unavailable ? "cursor-not-allowed text-muted-foreground" : "cursor-pointer")} data-wsl-distro={choice.distro} data-wsl-distro-state={choice.kind}>
      <RadioGroupItem value={choice.value} disabled={unavailable || disabled} aria-label={choice.deviceName} aria-describedby={reason ? note : undefined} className="mt-xxs" />
      <span className="flex min-w-0 flex-1 flex-col gap-xxs">
        <span className={cn("break-all text-subhead", unavailable ? "text-muted-foreground" : "text-foreground")}>{choice.deviceName}</span>
        <span className="break-keep text-body text-muted-foreground">{t("devices.wsl.thisPc")}</span>
      </span>
      {reason ? (
        <span id={note} className="flex max-w-1/2 shrink-0 flex-col gap-xxs text-right text-body text-muted-foreground">
          <span className="break-words break-keep" data-wsl-distro-note={choice.distro}>{reason}</span>
          {/* The command the reason ends on, under it, wrapping only between its words. */}
          {command ? <span className="break-words font-mono text-subtle-foreground" data-wsl-command={choice.distro}>{command}</span> : null}
        </span>
      ) : null}
    </label>
  );
}

/** What Hide puts on the device: three lines, said where the person agrees to them (B53). */
export function DeviceInstallTerms({ helperRoot, cliDir }: { helperRoot: string | null; cliDir: string | null }) {
  const { t } = useInterfaceTranslation();
  return (
    <ul className="list-disc space-y-xs pl-md text-body text-subtle-foreground" data-add-device-installs="true">
      <li>{t("devices.install.helper", { root: helperRoot ?? t("devices.helperFolder"), cliDir: cliDir ?? t("devices.commandFolder") })}</li>
      <li>{t("devices.install.hooks")}</li>
      <li>{t("devices.install.remove")}</li>
    </ul>
  );
}

/** No Host to choose: only what to add to ~/.ssh/config, never a form (B52). */
function NoHosts() {
  const { t } = useInterfaceTranslation();
  return (
    <div className="space-y-sm" data-ssh-hosts-state="empty">
      <Note data-ssh-hosts-hint="true">{t("devices.noHosts")}</Note>
      <pre className="overflow-x-auto rounded-md border border-border bg-card px-md py-sm font-mono text-body text-subtle-foreground">{HOST_EXAMPLE}</pre>
    </div>
  );
}
