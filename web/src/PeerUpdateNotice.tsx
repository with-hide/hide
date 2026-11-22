// The notice on the computer whose Hide is too old to connect to a paired
// one (PRD device-connect-iroh B22, D-40): which computer waits, and Update
// now. It is also where another computer's ask is followed: a computer that
// opens this one may ask it to update, and then the app updates and
// restarts by itself. Only the desktop app updates itself, on its own
// window; a browser tab, the phone and another computer's view of this one
// show nothing.

import { useState } from "react";
import { Button } from "./components/ui/button";
import { Toast } from "./components/ui/toast";
import { useInterfaceTranslation } from "./i18n/client";
import { deviceConnectOffered, ownWindow, pairedMachines } from "./pairing";
import { useShellStore } from "./store";
import { offersUpdate, updateNow, useAskedUpdate, useUpdateStatus } from "./updates";

export function PeerUpdateNotice() {
  const { t } = useInterfaceTranslation();
  const status = useUpdateStatus();
  // A build that does not offer connections between two Hides has no paired
  // computer to wait for, and follows no computer's ask.
  const own = useShellStore((s) => deviceConnectOffered(s.daemon) && ownWindow(s.pairing));
  const peers = useShellStore((s) => pairedMachines(s.rest));
  // The computer whose notice was closed here; another one shows it again.
  const [closed, setClosed] = useState<string | null>(null);
  const waiting = own && status !== null ? peers.filter((peer) => peer.update === "here" || peer.update === "here_asked") : [];
  useAskedUpdate(waiting.some((peer) => peer.update === "here_asked"));
  const peer = waiting[0];
  if (!peer || peer.key === closed) return null;
  const phase = status?.phase.kind;
  const offers = offersUpdate(status);
  return (
    <Toast
      level="important"
      data-peer-update-notice={peer.device_id}
      onDismiss={() => setClosed(peer.key)}
      dismissLabel={t("workspace.dismiss")}
      actions={
        offers ? (
          <Button variant="text" size="inline" data-peer-update-now="true" onClick={() => updateNow(status)}>{t("pairing.action.updateHere")}</Button>
        ) : phase === "off" ? null : (
          <span className="text-muted-foreground">{t("pairing.notice.updating")}</span>
        )
      }
    >
      {t("pairing.notice.updateHere", { name: peer.name })}
    </Toast>
  );
}
