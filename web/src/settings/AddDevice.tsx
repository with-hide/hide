// The Add device dialog's body (PRD settings-cleanup D-19, B50 to B53): the
// account's ssh config Hosts to choose from, each with the address ssh
// resolves it to; a chosen Host fills the name, and Add stores only that name
// and the alias. Username, port and key stay in ssh's own config, so there is
// no input for them. On Windows this PC's WSL distributions are listed first,
// under the same consent (PRD wsl-device B4); a distribution's device takes
// the name the core gives it (D-20). The list is hided's
// (`status.ssh_hosts`), asked for when the dialog opens.

import { useEffect, useId, useState } from "react";
import type { Actions } from "../actions";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { RadioGroup, RadioGroupItem } from "../components/ui/radio-group";
import { Disclosure, Group, Note, Row } from "../components/settings-rows";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { deviceIdFor, socketProblem } from "../settings";
import type { Device } from "../snapshot";
import { chosenDistro, defaultDeviceName, hostChoices, pickableAliases, wslChoices, type HostChoice, type WslChoice } from "../sshHosts";
import { useShellStore } from "../store";
import { coreErrorText } from "../coreError";
import { useRefusalSince } from "../errorSince";

const HOST_EXAMPLE = "Host studio\n  HostName studio.local\n  User you";

export function AddDevice({ actions, devices, helperRoot, cliDir }: { actions: Actions; devices: Device[]; helperRoot: string | null; cliDir: string | null }) {
  const { t } = useInterfaceTranslation();
  const hosts = useShellStore((s) => s.rest?.status?.ssh_hosts);
  const [alias, setAlias] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [nameEdited, setNameEdited] = useState(false);
  const [socket, setSocket] = useState("");
  const [submitted, setSubmitted] = useState<{ id: string | null; distro: string | null; at: number } | null>(null);
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
  const added = submitted ? devices.some((device) => (submitted.distro !== null ? device.wsl_distro === submitted.distro : device.id === submitted.id)) : false;
  const pending = submitted !== null && !added && error === null;
  const blocked = pending || chosen === null || (distro === null && (!name.trim() || socketProblem(socket, t) !== null));
  const choose = (next: string) => {
    setAlias(next);
    // The name follows the Host until the person writes their own.
    if (!nameEdited && chosenDistro(next) === null) setName(defaultDeviceName(next));
  };
  // Adding is where the whole kit is agreed to, once (PRD device-parity
  // D-12): what Hide installs is under Advanced and the description above
  // says it, and there is one way to add.
  const submit = () => {
    if (blocked || chosen === null) return;
    if (distro !== null) {
      setSubmitted({ id: null, distro, at: Date.now() });
      actions.registerWslDevice(distro);
      return;
    }
    const id = deviceIdFor(chosen, devices.map((device) => device.id));
    setSubmitted({ id, distro: null, at: Date.now() });
    actions.registerDevice(id, name.trim(), chosen, { hostConsent: true, herdrSocketPath: socket.trim() || null });
  };
  // Until hided has answered there is nothing to choose; an answer with no Host says what to add (B52).
  const reading = hosts === undefined || ((hosts.state === "idle" || hosts.state === "loading") && hosts.hosts.length === 0 && distributions.length === 0);
  if (reading) return <Note tone="pending" data-ssh-hosts-state="loading">{t("devices.hostsLoading")}</Note>;
  const unreadable = hosts.wsl_problem === "unreadable" ? <Note tone="warn" data-wsl-problem="unreadable">{t("devices.wsl.unreadable")}</Note> : null;
  if (choices.length === 0 && distributions.length === 0) return <>{unreadable}<NoHosts /></>;
  return (
    <div className="space-y-md" data-add-device-form="true">
      <p className="break-keep text-body text-muted-foreground">{t("devices.addDescription")}</p>
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
          {distro === null ? (
          <Row label={t("devices.herdrSocket")} detail={socketProblem(socket, t) ? <Note tone="warn">{socketProblem(socket, t)}</Note> : <Note>{t("devices.socketDescription")}</Note>}>
            <Input
              mono
              value={socket}
              disabled={pending}
              placeholder={t("devices.defaultServer")}
              aria-label={t("devices.socketAria")}
              className="w-(--size-settings-control-w)"
              onChange={(event) => setSocket(event.target.value)}
              data-device-socket="true"
            />
          </Row>
          ) : null}
        </Disclosure>
      </Group>
      <div className="flex justify-end">
        <Button disabled={blocked} onClick={submit} data-add-device="true">
          {pending ? t("devices.adding") : t("common.add")}
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
        {command ? <span className="break-all font-mono text-body text-subtle-foreground" data-wsl-command={choice.distro}>{command}</span> : null}
      </span>
      {reason ? (
        <span id={note} className="max-w-1/2 shrink-0 break-words break-keep text-right text-body text-muted-foreground" data-wsl-distro-note={choice.distro}>
          {reason}
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
