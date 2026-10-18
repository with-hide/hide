// Stored drafts no open tab stands for (PRD S5.5 B10-B12, B26, B44): a
// daemon restart that lost its tabs, another device's or another host's
// documents, and drafts from before drafts named their device. Each can be
// opened where it belongs, exported, or discarded on purpose; none goes on
// its own. A draft whose origin is unverified is never opened on a device it
// cannot be proven to belong to.

import { useState } from "react";
import type { Actions } from "./actions";
import { adoptNodeDrafts, allBuffers, deleteBufferId, flushBuffer, identity, recoveryBuffers, tabBufferKey, type StoredBuffer } from "./buffers";
import { Button } from "./components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogHeader, DialogTitle } from "./components/ui/dialog";
import { Hint } from "./components/ui/tooltip";
import { frontDeviceId } from "./devices";
import { machineName, windowDeviceId } from "./screenMachine";
import { catalogWorkspaces, frontCheckout, localDeviceId, type SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import type { TFunction } from "i18next";
import { useInterfaceTranslation } from "./i18n/client";
import { formatDateTime } from "./i18n/format";
import type { InterfaceLanguage } from "./i18n/locale";

/** Re-reads the stored drafts and keeps the ones no open tab stands for. */
export async function refreshRecoveryDrafts(): Promise<void> {
  const state = useShellStore.getState();
  const host = state.daemon?.host_id ?? null;
  const keys = (state.editor?.tabs ?? [])
    .filter((tab) => tab.kind === "file")
    .map((tab) => tabBufferKey(host, state.rest, tab))
    .filter((key) => key !== null);
  await Promise.all(keys.map((key) => flushBuffer(key)));
  if (host) await adoptNodeDrafts(host, localDeviceId(state.rest));
  const open = new Set(keys.map((key) => identity(key)));
  useShellStore.getState().setRecoveryDrafts(recoveryBuffers(await allBuffers(), open));
}

export type DraftPlace =
  | { kind: "front" }
  | { kind: "device"; device: string; label: string }
  | { kind: "checkout"; workspaceId: string; checkoutId: string; label: string }
  | { kind: "none"; reason: string };

/**
 * Where a draft can be opened from here. It opens only in its own checkout
 * on its own device of this daemon host; a draft from before drafts named
 * their device may be claimed by this machine's own checkout at that root and
 * nowhere else (B11).
 */
export function draftPlace(draft: StoredBuffer, host: string | null, rest: SnapshotRest | null, t: TFunction<"translation">): DraftPlace {
  if (!draft.root) return { kind: "none", reason: t("documents.draftBeforeCheckout") };
  const device = draft.device ?? localDeviceId(rest);
  if (draft.host !== null && draft.host !== host) return { kind: "none", reason: t("documents.draftOtherHost") };
  const checkout = catalogWorkspaces(rest)
    .filter((workspace) => workspace.device_id === device)
    .flatMap((workspace) => workspace.checkouts)
    .find((row) => row.path === draft.root);
  if (!checkout) return { kind: "none", reason: t("documents.draftCheckoutMissing", { path: draft.root, device: deviceLabel(rest, device, t) }) };
  const focusedDevice = frontDeviceId(rest);
  if (focusedDevice !== device) return { kind: "device", device, label: deviceLabel(rest, device, t) };
  if (frontCheckout(rest)?.id !== checkout.id) {
    return { kind: "checkout", workspaceId: checkout.workspace_id, checkoutId: checkout.id, label: checkout.label };
  }
  return { kind: "front" };
}

function deviceLabel(rest: SnapshotRest | null, device: string, t: TFunction<"translation">): string {
  if (device === windowDeviceId(rest?.navigator?.devices)) return t("documents.thisMachine");
  return machineName(rest?.navigator?.devices, device, t);
}

function origin(draft: StoredBuffer, host: string | null, rest: SnapshotRest | null, t: TFunction<"translation">): string {
  if (draft.host === null) return t("documents.originUnverified");
  if (draft.host !== host) return t("documents.otherHost");
  return deviceLabel(rest, draft.device ?? localDeviceId(rest), t);
}

function exportContents(draft: StoredBuffer) {
  const url = URL.createObjectURL(new Blob([draft.contents], { type: "text/plain;charset=utf-8" }));
  const link = window.document.createElement("a");
  link.href = url;
  link.download = `${draft.path.split("/").pop() || "draft"}.draft`;
  link.click();
  URL.revokeObjectURL(url);
}

export function DraftRecoveryLine({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const count = useShellStore((s) => s.recoveryDrafts.length);
  const [open, setOpen] = useState(false);
  if (count === 0) return null;
  return (
    <>
      <div role="status" data-draft-recovery={count} className="flex items-center gap-md border-b border-border bg-card px-md py-xs text-caption text-warning">
        <Hint label={t("documents.recovery", { count })}>
          <span className="min-w-0 flex-1 truncate">
            {t("documents.recovery", { count })}
          </span>
        </Hint>
        <button type="button" className="text-foreground underline" data-draft-recovery-review="true" onClick={() => setOpen(true)}>
          {t("documents.reviewDrafts")}
        </button>
      </div>
      {open ? <DraftRecoverySheet actions={actions} onClose={() => setOpen(false)} /> : null}
    </>
  );
}

function DraftRecoverySheet({ actions, onClose }: { actions: Actions; onClose: () => void }) {
  const { t, i18n } = useInterfaceTranslation();
  const drafts = useShellStore((s) => s.recoveryDrafts);
  const host = useShellStore((s) => s.daemon?.host_id ?? null);
  const rest = useShellStore((s) => s.rest);
  const [confirming, setConfirming] = useState<string | null>(null);
  return (
    <Dialog open onOpenChange={(next) => { if (!next) onClose(); }}>
      <DialogContent data-draft-recovery-sheet="true" className="max-h-(--size-settings-sheet-h-max)">
        <DialogHeader>
          <DialogTitle>{t("documents.unsavedDrafts")}</DialogTitle>
        </DialogHeader>
        <DialogBody>
          {drafts.length === 0 ? <p className="text-caption text-muted-foreground">{t("documents.noDrafts")}</p> : null}
          <ul className="flex flex-col gap-sm">
            {drafts.map((draft) => {
              const place = draftPlace(draft, host, rest, t);
              // The id joins its parts with NUL, which a DOM attribute selector
              // cannot match; the attributes carry it URI-encoded.
              const tag = encodeURIComponent(draft.id);
              return (
                <li key={draft.id} className="flex flex-col gap-xxs border-b border-border pb-sm" data-draft={tag}>
                  <Hint label={draft.path}>
                    <span className="truncate font-mono text-caption text-foreground">{draft.path}</span>
                  </Hint>
                  <span className="text-caption text-muted-foreground">
                    {origin(draft, host, rest, t)} · {formatDateTime(i18n.language as InterfaceLanguage, draft.updated_at, { year: "numeric", month: "numeric", day: "numeric", hour: "numeric", minute: "numeric", second: "numeric" })}
                    {place.kind === "none" ? ` · ${t("documents.cannotOpen", { reason: place.reason })}` : ""}
                  </span>
                  <span className="flex flex-wrap gap-sm">
                    {place.kind === "front" ? (
                      <Button size="sm" data-draft-open={tag} onClick={() => { actions.openFile(draft.path, false); onClose(); }}>{t("common.open")}</Button>
                    ) : place.kind === "device" ? (
                      <Button size="sm" data-draft-show-device={tag} onClick={() => actions.focusDevice(place.device)}>{t("documents.showPlace", { place: place.label })}</Button>
                    ) : place.kind === "checkout" ? (
                      <Button size="sm" data-draft-show-checkout={tag} onClick={() => actions.focusCheckout(place.workspaceId, place.checkoutId)}>{t("documents.showPlace", { place: place.label })}</Button>
                    ) : null}
                    <Button variant="ghost" size="sm" data-draft-export={tag} onClick={() => exportContents(draft)}>{t("documents.export")}</Button>
                    {confirming === draft.id ? (
                      <Button
                        variant="destructive"
                        size="sm"
                        data-draft-discard-confirm={tag}
                        onClick={() => {
                          setConfirming(null);
                          void deleteBufferId(draft.id).then(refreshRecoveryDrafts);
                        }}
                      >
                        {t("documents.discardPermanently")}
                      </Button>
                    ) : (
                      <Button variant="ghost" size="sm" data-draft-discard={tag} onClick={() => setConfirming(draft.id)}>{t("documents.discard")}</Button>
                    )}
                  </span>
                </li>
              );
            })}
          </ul>
        </DialogBody>
      </DialogContent>
    </Dialog>
  );
}
