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
// its own runs there and, when none does, whether one can be started there
// (PRD device-connect-iroh B7; `addWay.ts`). It is connected as a server
// where it can be (operator decision 2026-11-28): paired with the Hide it
// runs, which installs nothing there, or added and switched with a Hide
// started there; one that cannot be says why in one line and is added over
// SSH, which stays one small row under the server card for any Host. A
// chosen distribution is asked the same through wsl.exe (D-27), which
// starts it when it is off (B43): one whose own Hide runs is connected to,
// any other is added and used over WSL until its row connects it.

import { useEffect, useId, useState } from "react";
import type { Actions } from "../actions";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { RadioGroup, RadioGroupItem } from "../components/ui/radio-group";
import { Disclosure, Group, Note, Row, Status } from "../components/settings-rows";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import type { Device, DeviceSwitchStep } from "../snapshot";
import { chosenDistro, defaultDeviceName, hostChoices, pickableAliases, wslChoices, type HostChoice, type WslChoice } from "../sshHosts";
import { useShellStore } from "../store";
import { coreErrorText } from "../coreError";
import { isPairLink, refusalKey, sshLookLine, type PairingResult } from "../pairing";
import { useOnScreen } from "../lib/useOnScreen";
import { useUiStore } from "../ui";
import { addWay, systemName, type AddWay, type HostAnswer } from "./addWay";
import { Headline, Picture, SwitchSteps, WayCard } from "./DeviceWays";
import type { SwitchOutcome } from "./switchWays";
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

export function AddDevice({ actions, devices, helperRoot, cliDir, onClose, onShowDevice }: { actions: Actions; devices: Device[]; helperRoot: string | null; cliDir: string | null; onClose: () => void; onShowDevice?: (deviceId: string) => void }) {
  const { t } = useInterfaceTranslation();
  const hosts = useShellStore((s) => s.rest?.status?.ssh_hosts);
  const answered = useShellStore((s) => s.pairingResult);
  // What the chosen Host or distribution answered, and the press that
  // connects it as a server: a pairing with the Hide it runs, or a Hide
  // started there (`add_server`).
  const [look, setLook] = useState<{ alias: string; request: string; answer: HostAnswer } | null>(null);
  const [pairing, setPairing] = useState<{ request: string; alias: string; verb: "pair" | "add_server"; refused: string | null } | null>(null);
  useEffect(() => {
    if (!answered?.request_id) return;
    if ((answered.verb === "look_ssh" || answered.verb === "look_wsl") && answered.request_id === look?.request && !look.answer.answered) {
      const answer: HostAnswer = answered.ok ? { answered: true, looked: answered.looked ?? null, system: answered.system ?? null, leaves: answered.leaves ?? null } : { answered: true, looked: null, system: null, leaves: null };
      setLook({ ...look, answer });
    }
    if (answered.request_id !== pairing?.request || pairing.refused !== null) return;
    const refused = () => setPairing({ ...pairing, refused: answered.reason ?? "" });
    if (answered.verb === "pair_ssh" || answered.verb === "pair_wsl") {
      if (answered.ok) onClose();
      else refused();
    } else if (answered.verb === "add_server") {
      // The device was added over SSH and not switched: its row, where the
      // switch can be pressed again, says why; a machine never added says
      // it here.
      const device = devices.find((row) => row.id === answered.device_id) ?? devices.find((row) => row.ssh_alias === pairing.alias);
      if (answered.ok && answered.switch === "done") onClose();
      else if (!device || !onShowDevice) refused();
      else {
        const outcome = serverAddOutcome(answered);
        if (outcome) useUiStore.getState().setSwitchOutcome(device.id, outcome);
        setPairing(null);
        onShowDevice(device.id);
      }
    }
  }, [answered, look, pairing, devices, onClose, onShowDevice]);
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
  // An added device closes the dialog: its row is listed now.
  useEffect(() => {
    if (added) onClose();
  }, [added, onClose]);
  const answer: HostAnswer = look !== null && look.alias === chosen ? look.answer : { answered: false };
  // A Host's way (operator decision 2026-11-28); a distribution keeps the
  // form it had: connected to when its own Hide runs, else added.
  const way = chosen === null ? null : distro === null ? addWay(answer) : null;
  const looked = answer.answered ? answer.looked : null;
  const lookLine = distro === null ? null : sshLookLine(looked);
  const connects = way?.kind === "connect" || (distro !== null && looked?.state === "ready");
  const running = pairing !== null && pairing.refused === null;
  const pending = (submitted !== null && !added && error === null) || running;
  // Until the Host answered, a press would add a machine that runs its own
  // Hide as a helper device: the press waits for the answer.
  const looking = chosen !== null && !answer.answered;
  // A press that adds a device names it; a pairing keeps the name that machine gives itself.
  const named = distro !== null || connects || name.trim() !== "";
  const blocked = pending || looking || chosen === null || !named;
  // An added machine's row opens that device's row, where its 서버로 연결 is, when the dialog sits on Settings › Devices.
  const shown = (device: Device | undefined) => (device && onShowDevice ? () => onShowDevice(device.id) : null);
  const choose = (next: string) => {
    setAlias(next);
    setPairing(null);
    const picked = chosenDistro(next);
    // The name follows the Host until the person writes their own; a
    // distribution's is the core's.
    if (picked === null && !nameEdited) setName(defaultDeviceName(next));
    const request = `look-${Date.now()}`;
    setLook({ alias: next, request, answer: { answered: false } });
    if (picked === null) actions.pairing("look_ssh", { alias: next, request_id: request });
    else actions.pairing("look_wsl", { distro: picked, request_id: request });
  };
  // Adding over SSH or wsl.exe is where the whole kit is agreed to, once
  // (PRD device-parity D-12): what Hide installs is under Advanced and the
  // description above says it. A device added so is used over that login
  // until the operator connects it as a server from its row (operator
  // decision 2026-11-24): the Add never ends without the device.
  const addOverLogin = () => {
    if (pending || looking || chosen === null) return;
    if (distro !== null) {
      setSubmitted({ alias: null, distro, at: Date.now() });
      actions.registerWslDevice(distro);
    } else if (name.trim()) {
      setSubmitted({ alias: chosen, distro: null, at: Date.now() });
      actions.registerDevice(name.trim(), chosen, { hostConsent: true });
    }
  };
  const submit = () => {
    if (blocked || chosen === null) return;
    const request = `pair-${Date.now()}`;
    if (connects) {
      setPairing({ request, alias: chosen, verb: "pair", refused: null });
      if (distro !== null) actions.pairing("pair_wsl", { distro, request_id: request });
      else actions.pairing("pair_ssh", { alias: chosen, request_id: request });
      return;
    }
    // The press is the consent to Hide's helper and to a Hide started there, as the button says.
    if (way?.kind === "install") {
      setPairing({ request, alias: chosen, verb: "add_server", refused: null });
      actions.pairing("add_server", { alias: chosen, name: name.trim(), request_id: request });
      return;
    }
    addOverLogin();
  };
  // Until hided has answered there is nothing to choose; an answer with no Host says what to add (B52).
  const reading = hosts === undefined || ((hosts.state === "idle" || hosts.state === "loading") && hosts.hosts.length === 0 && distributions.length === 0);
  if (reading) return <Note tone="pending" data-ssh-hosts-state="loading">{t("devices.hostsLoading")}</Note>;
  const unreadable = hosts.wsl_problem === "unreadable" ? <Note tone="warn" data-wsl-problem="unreadable">{t("devices.wsl.unreadable")}</Note> : null;
  if (choices.length === 0 && distributions.length === 0) return <>{unreadable}<NoHosts /></>;
  const server = way?.kind === "connect" || way?.kind === "install";
  const primary = way === null ? (connects ? t("pairing.add.connect") : t("common.add")) : way.kind === "install" ? t("pairing.add.installConnect") : way.kind === "ssh" ? t("pairing.add.connect") : t("devices.ways.server");
  const shownName = name.trim() || (chosen ?? "");
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
          <WslRow key={choice.value} choice={choice} disabled={pending} onShow={shown(choice.kind === "added" ? devices.find((device) => device.wsl_distro === choice.distro) : undefined)} />
        ))}
        {choices.map((choice) => (
          <HostRow key={choice.alias} choice={choice} disabled={pending} onShow={shown(choice.kind === "added" ? addedHost(devices, choice.alias, choice.name) : undefined)} />
        ))}
      </RadioGroup>
      {unreadable}
      {hosts.truncated ? <Note>{t("devices.hostsTruncated", { count: hosts.hosts.length })}</Note> : null}
      <Note data-ssh-hosts-hint="true">{t("devices.notListed")}</Note>
      {looking ? <Note tone="pending" data-ssh-look="looking">{t("pairing.ssh.looking")}</Note> : null}
      {lookLine ? <Status tone={lookLine.tone} data-ssh-look={looked?.state}>{t(lookLine.key)}</Status> : null}
      {way?.kind === "ssh" && way.reason ? <Note tone="warn" data-add-way-reason={looked?.state ?? ""}>{t(way.reason)}</Note> : null}
      {way && way.kind !== "looking" && chosen !== null ? <HostWay way={way} name={shownName} running={running && pairing?.verb === "add_server"} step={devices.find((device) => device.ssh_alias === chosen)?.switch_step ?? null} /> : null}
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
        <Button variant="secondary" onClick={onClose} data-add-device-cancel="true">
          {t("common.cancel")}
        </Button>
        <Button disabled={blocked} onClick={submit} data-add-device={connects ? "connect" : way?.kind === "install" ? "server" : "true"}>
          {running ? t("pairing.ssh.connecting") : pending ? t("devices.adding") : primary}
        </Button>
      </div>
      {/* The other way, kept as one small row under the main card (operator decision 2026-11-28). */}
      {server && !running ? (
        <div className="flex items-start gap-md border-t border-border pt-sm" data-add-device-direct="true">
          <p className="min-w-0 flex-1 break-keep text-body text-muted-foreground">
            <span className="font-medium text-foreground">{t("devices.ways.ssh")}</span>
            {" · "}
            {t("pairing.add.directLine")}
          </p>
          <Button variant="secondary" size="sm" className="shrink-0" disabled={pending || !name.trim()} onClick={addOverLogin} data-add-device-over-ssh="true">
            {t("pairing.add.directGo")}
          </Button>
        </div>
      ) : null}
    </div>
  );
}

/**
 * How a pressed `add_server` that did not switch the device ends on its
 * row: another computer switched it first, or a step failed. A machine the
 * rule left, or one that has not connected yet, says so on the row's own
 * read and line.
 */
function serverAddOutcome(answered: PairingResult): SwitchOutcome | null {
  if (answered.ok) return answered.switch === "held" ? { kind: "held", by: answered.by ?? "", terminals: answered.terminals ?? null } : null;
  if (answered.reason === "added_not_connected" || answered.reason === "switch_running") return null;
  return { kind: "failed", reason: answered.reason ?? "" };
}

/**
 * The way a chosen Host is connected, as the device row draws it
 * (`DeviceWays`): the server card with `추천`, what this computer's sleep
 * does, where Hide runs and what the Host was read as; while a Hide is
 * started there, the switch's steps in their place. A Host that cannot be
 * a server shows the SSH card instead.
 */
function HostWay({ way, name, running, step }: { way: Exclude<AddWay, { kind: "looking" }>; name: string; running: boolean; step: DeviceSwitchStep | null }) {
  const { t } = useInterfaceTranslation();
  const [ref, playing] = useOnScreen<HTMLDivElement>();
  const server = way.kind !== "ssh";
  const system = way.kind === "ssh" ? null : systemName(way.system);
  const read = way.kind === "connect" ? t("pairing.add.hasHide") : way.kind === "install" ? t("pairing.add.noHide", { device: name }) : null;
  return (
    <div ref={ref} className="dw-ways" data-playing={playing ? "true" : "false"} data-add-way={way.kind}>
      <WayCard
        title={t(server ? "devices.ways.server" : "devices.ways.ssh")}
        now={false}
        mark={server ? t("pairing.add.recommended") : undefined}
        lit={server}
        foot={read ? <p className="break-keep text-caption text-muted-foreground" data-add-way-read={way.kind}>{system ? `${system} · ${read}` : read}</p> : null}
      >
        {server ? <Headline lead={t("devices.ways.serverHead")} turn={t("devices.ways.serverHeadGo", { device: name })} tone="ok" /> : <Headline lead={t("devices.ways.directHead")} turn={t("devices.ways.directHeadStop")} tone="warn" />}
        {running ? <SwitchSteps name={name} current={step ?? "checking"} /> : <Picture way={server ? "server" : "direct"} name={name} />}
      </WayCard>
    </div>
  );
}

/** One Host: chosen with a radio, or dimmed with why it cannot be (B51, B52). */
function HostRow({ choice, disabled, onShow }: { choice: HostChoice; disabled: boolean; onShow: (() => void) | null }) {
  const { t } = useInterfaceTranslation();
  const note = useId();
  const unavailable = choice.kind !== "available";
  const reason = choice.kind === "added" ? t("pairing.ssh.addedOverSsh", { name: choice.name }) : choice.kind === "unresolved" ? t(choice.reason) : null;
  const address = choice.kind === "unresolved" ? null : choice.address;
  // A row with a button of its own is no label, which would hold two controls.
  const Outer = onShow ? "div" : "label";
  return (
    <Outer className={cn("flex items-start gap-md px-md py-sm", unavailable ? "cursor-not-allowed text-muted-foreground" : "cursor-pointer")} data-ssh-host={choice.alias} data-ssh-host-state={choice.kind}>
      <RadioGroupItem value={choice.alias} disabled={unavailable || disabled} aria-label={choice.alias} aria-describedby={reason ? note : undefined} className="mt-xxs" />
      <span className="flex min-w-0 flex-1 flex-col gap-xxs">
        <span className={cn("break-all text-subhead", unavailable ? "text-muted-foreground" : "text-foreground")}>{choice.alias}</span>
        {address ? <span className="break-all font-mono text-body text-muted-foreground">{address}</span> : null}
      </span>
      {reason ? (
        <span className="flex max-w-1/2 shrink-0 flex-col items-end gap-xxs text-right text-body text-muted-foreground">
          <span id={note} className="break-words break-keep" data-ssh-host-note={choice.alias}>
            {reason}
          </span>
          <AddedWay onShow={onShow} data-ssh-host-server={choice.alias} />
        </span>
      ) : null}
    </Outer>
  );
}

/**
 * The way from a machine this computer already reaches, over SSH or wsl.exe,
 * to Hide of its own there: that device's 서버로 연결, which this opens on its
 * row in Settings › Devices. Adding the machine again would only answer with
 * the helper this computer runs there (with-hide/hide issue 3007).
 */
function AddedWay({ onShow, ...data }: { onShow: (() => void) | null } & Record<`data-${string}`, string>) {
  const { t } = useInterfaceTranslation();
  if (!onShow) return null;
  return (
    <Button variant="text" size="inline" onClick={onShow} {...data}>
      {t("pairing.ssh.openServer")}
    </Button>
  );
}

/** One WSL distribution of this PC: chosen like a Host, or dimmed with why it cannot be (PRD wsl-device B4, B28). */
function WslRow({ choice, disabled, onShow }: { choice: WslChoice; disabled: boolean; onShow: (() => void) | null }) {
  const { t } = useInterfaceTranslation();
  const note = useId();
  const unavailable = choice.kind !== "available";
  const reason = choice.kind === "added" ? t("devices.hostAdded", { name: choice.name }) : choice.kind === "unavailable" ? t(choice.reason) : null;
  const command = choice.kind === "unavailable" ? choice.command : null;
  const Outer = onShow ? "div" : "label";
  return (
    <Outer className={cn("flex items-start gap-md px-md py-sm", unavailable ? "cursor-not-allowed text-muted-foreground" : "cursor-pointer")} data-wsl-distro={choice.distro} data-wsl-distro-state={choice.kind}>
      <RadioGroupItem value={choice.value} disabled={unavailable || disabled} aria-label={choice.deviceName} aria-describedby={reason ? note : undefined} className="mt-xxs" />
      <span className="flex min-w-0 flex-1 flex-col gap-xxs">
        <span className={cn("break-all text-subhead", unavailable ? "text-muted-foreground" : "text-foreground")}>{choice.deviceName}</span>
        <span className="break-keep text-body text-muted-foreground">{t("devices.wsl.thisPc")}</span>
      </span>
      {reason ? (
        <span className="flex max-w-1/2 shrink-0 flex-col items-end gap-xxs text-right text-body text-muted-foreground">
          <span id={note} className="flex flex-col gap-xxs">
            <span className="break-words break-keep" data-wsl-distro-note={choice.distro}>{reason}</span>
            {/* The command the reason ends on, under it, wrapping only between its words. */}
            {command ? <span className="break-words font-mono text-subtle-foreground" data-wsl-command={choice.distro}>{command}</span> : null}
          </span>
          <AddedWay onShow={onShow} data-wsl-distro-server={choice.distro} />
        </span>
      ) : null}
    </Outer>
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

/** The device an added Host names: the one dialed by that alias, or the one the core named for reaching the same machine. */
function addedHost(devices: Device[], alias: string, name: string): Device | undefined {
  return devices.find((device) => device.ssh_alias === alias) ?? devices.find((device) => device.kind === "remote" && device.label === name);
}
