// One registration Add a project sends, and the answer it waits for. Every
// way of adding (Browse folder, a device's path, Create new project) sends one
// `create_workspace` and reads the answer the same way: the registrations
// growing closes the dialog, and a refusal (hided's `path_refused`, the core's
// error, worded by its kind, or the one the shell can see itself) stays inside
// it, naming the folder.

import { useEffect, useState } from "react";
import { folderLabel, refusalText } from "./addProject";
import { useInterfaceTranslation } from "./i18n/client";
import { Status } from "./components/settings-rows";
import type { WorkspaceRegistration } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { coreErrorText } from "./coreError";
import { useErrorSince } from "./errorSince";

/**
 * A folder sent as `create_workspace`, with the ids of its device's projects
 * then: a new id is the answer, whatever else was removed meanwhile, and
 * whichever spelling (canonical, or a device's own) the registration keeps.
 */
type Sent = { device: string; path: string; at: number; known: ReadonlySet<string> };

export type Shown = { path: string; reason: string; text: string };

export type Registration = {
  sent: Sent | null;
  /** The refusal the dialog shows, whoever answered it. */
  shown: Shown | null;
  /** Sent and not answered yet. */
  pending: boolean;
  /** Sends one registration through `dispatch`, forgetting any earlier answer. */
  send: (device: string, path: string, dispatch: () => void) => void;
  /** A refusal the shell tells itself, before anything is sent. */
  refuse: (path: string, reason: string) => void;
  /** Forgets what was sent and answered, as a new host or view does. */
  reset: () => void;
};

export function useRegistration(registrations: readonly WorkspaceRegistration[]): Registration {
  const { t } = useInterfaceTranslation();
  const pathRefusal = useShellStore((s) => s.pathRefusal);
  const [sent, setSent] = useState<Sent | null>(null);
  const [shellRefusal, setShellRefusal] = useState<{ path: string; reason: string } | null>(null);
  const coreError = useErrorSince(sent?.at ?? null, ["workspace.create", "workspace.remove_in_flight", "workspace.control_unavailable"]);
  const added = sent !== null && registrations.some((row) => row.device_id === sent.device && !sent.known.has(row.id));

  const hidedReason = sent && pathRefusal?.kind === "create_workspace" && pathRefusal.path === sent.path ? pathRefusal.reason : null;
  const shown = shellRefusal
    ? { ...shellRefusal, text: refusalText(shellRefusal.reason, t) }
    : sent && hidedReason
      ? { path: sent.path, reason: hidedReason, text: refusalText(hidedReason, t) }
      : sent && coreError
        ? coreError.kind === "workspace.create_registered"
          ? { path: sent.path, reason: "already_registered", text: refusalText("already_registered", t) }
          : { path: sent.path, reason: coreError.kind, text: coreErrorText(coreError, t, "coreError.project.addFailed") }
        : null;

  // The project appearing is the answer; the dialog closes on it.
  useEffect(() => {
    if (added) useUiStore.getState().closeOverlay("add_project");
  }, [added]);

  return {
    sent,
    shown,
    pending: sent !== null && shown === null,
    send: (device, path, dispatch) => {
      setShellRefusal(null);
      useShellStore.getState().clearPathRefusal();
      setSent({ device, path, at: Date.now(), known: new Set(registrations.filter((row) => row.device_id === device).map((row) => row.id)) });
      dispatch();
    },
    refuse: (path, reason) => {
      setSent(null);
      setShellRefusal({ path, reason });
    },
    reset: () => {
      setSent(null);
      setShellRefusal(null);
    },
  };
}

/** `Adding <folder>…` while a registration is pending, then the refusal naming the folder. */
export function RegistrationStatus({ registration }: { registration: Registration }) {
  const { t } = useInterfaceTranslation();
  const { sent, shown, pending } = registration;
  return (
    <>
      {pending && sent ? (
        <Status tone="pending" data-registration-pending="true">
          {t("registration.adding", { folder: folderLabel(sent.path) })}
        </Status>
      ) : null}
      {shown ? (
        <div role="alert" className="flex min-w-0 flex-col gap-xxs" data-registration-reason={shown.reason} data-registration-path={shown.path}>
          <Status tone="error">{shown.text}</Status>
          <span className="break-all font-mono text-caption text-muted-foreground">{shown.path}</span>
        </div>
      ) : null}
    </>
  );
}
