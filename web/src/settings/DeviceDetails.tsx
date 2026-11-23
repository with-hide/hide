import { Dialog, DialogBody, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "../components/ui/dialog";
import { Row, Status, Value } from "../components/settings-rows";
import { useInterfaceTranslation } from "../i18n/client";
import { requireInterfaceLanguage } from "../i18n/locale";
import { directionKey, peerRouteKey, peerState, type Peer } from "../pairing";
import { hostLine } from "../settings";
import { machineName } from "../screenMachine";
import type { Device } from "../snapshot";
import { useShellStore } from "../store";
import { DeviceTest } from "./DeviceRow";
import { KitParts } from "./MachineKit";

/**
 * Connection details (PRD settings-cleanup B55; operator review 2026-11-23):
 * what a device row no longer carries. The SSH alias, the helper with its
 * folder, program and the host key the consent is bound to, the `hide`
 * command's folder, what the connection and the helper last said, the
 * latest test with its stages, and every part of Hide's kit with where it
 * is. This is the one place a working kit is listed.
 */
export function DeviceDetails({ device, onClose }: { device: Device; onClose: () => void }) {
  const { t, i18n } = useInterfaceTranslation();
  const host = device.host;
  const name = useShellStore((s) => machineName(s.rest?.navigator?.devices, device.id, t));
  const remote = device.kind === "remote";
  const helper = remote ? hostLine(host, t) : null;
  // The words the row leaves out: ssh's last line or the attempt's own, and the helper's.
  const said = [remote && device.state !== "ready" ? device.message : null, remote ? host?.message : null].filter((line, index, all): line is string => Boolean(line) && all.indexOf(line) === index);
  return (
    <Dialog open onOpenChange={(next) => { if (!next) onClose(); }}>
      <DialogContent data-device-details-dialog={device.id}>
        <DialogHeader>
          <DialogTitle>{t("devices.detailsTitle", { name })}</DialogTitle>
          <DialogDescription>{t("devices.detailsDescription")}</DialogDescription>
        </DialogHeader>
        <DialogBody className="space-y-md">
          <div className="divide-y divide-border rounded-md border border-border bg-card">
            {remote && device.ssh_alias ? <Row label={t("devices.detail.alias")}><Value data-device-detail="alias">{device.ssh_alias}</Value></Row> : null}
            {remote && helper ? <Row label={t("devices.detail.helper")}><Value mono={false}>{helper.text}</Value></Row> : null}
            {host?.helper_root ? <Row label={t("devices.detail.helperFolder")}><Value data-device-detail="helper-folder">{host.helper_root}</Value></Row> : null}
            {host?.helper_path ? <Row label={t("devices.detail.helperProgram")}><Value data-device-detail="helper-path">{host.helper_path}</Value></Row> : null}
            {host?.cli_dir ? <Row label={t("devices.detail.commandFolder")}><Value data-device-detail="command-folder">{host.cli_dir}</Value></Row> : null}
            {remote && host?.bound_identity ? <Row label={t("devices.detail.hostKey")}><Value data-device-detail="host-key">{host.bound_identity}</Value></Row> : null}
          </div>
          {said.length > 0 ? (
            <section className="space-y-xs" data-device-detail="message">
              <h4 className="text-body font-semibold text-subtle-foreground">{t("devices.detail.message")}</h4>
              {said.map((line) => (
                <p key={line} className="break-words font-mono text-caption text-muted-foreground" data-device-message={device.id}>{line}</p>
              ))}
            </section>
          ) : null}
          {device.test ? <DeviceTest test={device.test} language={requireInterfaceLanguage(i18n.language)} /> : null}
          <section className="space-y-xs">
            <h4 className="text-body font-semibold text-subtle-foreground">{t("devices.detail.kit")}</h4>
            <KitParts device={device} />
          </section>
        </DialogBody>
      </DialogContent>
    </Dialog>
  );
}

/**
 * A paired machine's connection details (D-86; operator review 2026-11-23):
 * the path the connection takes now (direct, Tailscale or relay) and who
 * opens whom, which its row no longer says.
 */
export function PeerDetails({ peer, onClose }: { peer: Peer; onClose: () => void }) {
  const { t } = useInterfaceTranslation();
  const route = peerRouteKey(peer);
  const state = peerState(peer);
  return (
    <Dialog open onOpenChange={(next) => { if (!next) onClose(); }}>
      <DialogContent data-peer-details-dialog={peer.device_id}>
        <DialogHeader>
          <DialogTitle>{t("devices.detailsTitle", { name: peer.name })}</DialogTitle>
        </DialogHeader>
        <DialogBody>
          <div className="divide-y divide-border rounded-md border border-border bg-card">
            <Row label={t("devices.detail.route")}>
              {route ? <Value mono={false} data-peer-route={peer.path ?? ""}>{t(route)}</Value> : <Status tone={state.tone === "ok" ? "live" : state.tone}>{t(state.key)}</Status>}
            </Row>
            <Row label={t("devices.detail.direction")}>
              <Value mono={false} data-peer-direction="true">{t(directionKey(peer))}</Value>
            </Row>
          </div>
        </DialogBody>
      </DialogContent>
    </Dialog>
  );
}
