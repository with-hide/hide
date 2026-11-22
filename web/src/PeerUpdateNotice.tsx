// The notice that a paired computer cannot be opened until one side updates
// (PRD device-connect-iroh B22, D-40), in the window's one toast stack and
// never in the strip over a screen. On the computer whose Hide is too old it
// says which computer waits, with Update now; it is also where another
// computer's ask is followed: a computer that opens this one may ask it to
// update, and then the app updates and restarts by itself. On the computer
// that waits it says the other one must update, with Ask to update. Only the
// desktop app updates itself, on its own window; a browser tab, the phone
// and another computer's view of this one show nothing.

import { useState } from "react";
import type { Actions } from "./actions";
import { Button } from "./components/ui/button";
import { Toast } from "./components/ui/toast";
import { useInterfaceTranslation } from "./i18n/client";
import { ownWindow, pairedMachines, type Peer } from "./pairing";
import { useShellStore } from "./store";
import { offersUpdate, updateNow, useAskedUpdate, useUpdateStatus } from "./updates";

export function PeerUpdateNotice({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const status = useUpdateStatus();
  const own = useShellStore((s) => ownWindow(s.pairing));
  const peers = useShellStore((s) => pairedMachines(s.rest));
  // The computer whose notice was closed here, and for which side; another one, or the other side, shows again.
  const [closed, setClosed] = useState<string | null>(null);
  const waiting = own && status !== null ? peers.filter((peer) => peer.update === "here" || peer.update === "here_asked") : [];
  useAskedUpdate(waiting.some((peer) => peer.update === "here_asked"));
  // An ask already sent leaves the row's line to say so, and the notice goes.
  const there = own ? peers.filter((peer) => peer.update === "there") : [];
  const here = waiting[0];
  if (here && `here:${here.key}` !== closed) return <UpdateHere peer={here} status={status} onClose={() => setClosed(`here:${here.key}`)} />;
  const peer: Peer | undefined = there[0];
  if (!peer || `there:${peer.key}` === closed) return null;
  return (
    <Toast
      level="important"
      data-peer-update-notice={peer.device_id}
      data-peer-update-side="there"
      onDismiss={() => setClosed(`there:${peer.key}`)}
      dismissLabel={t("workspace.dismiss")}
      actions={
        <Button variant="secondary" size="sm" data-peer-update-ask={peer.device_id} onClick={() => actions.pairing("update", { device: peer.key })}>
          {t("pairing.action.askUpdate")}
        </Button>
      }
    >
      {t("pairing.notice.updateThere", { name: peer.name })}
    </Toast>
  );
}

function UpdateHere({ peer, status, onClose }: { peer: Peer; status: ReturnType<typeof useUpdateStatus>; onClose: () => void }) {
  const { t } = useInterfaceTranslation();
  const phase = status?.phase.kind;
  return (
    <Toast
      level="important"
      data-peer-update-notice={peer.device_id}
      data-peer-update-side="here"
      onDismiss={onClose}
      dismissLabel={t("workspace.dismiss")}
      actions={
        offersUpdate(status) ? (
          <Button variant="tint" size="sm" data-peer-update-now="true" onClick={() => updateNow(status)}>{t("pairing.action.updateHere")}</Button>
        ) : phase === "off" ? null : (
          <span className="text-muted-foreground">{t("pairing.notice.updating")}</span>
        )
      }
    >
      {t("pairing.notice.updateHere", { name: peer.name })}
    </Toast>
  );
}
