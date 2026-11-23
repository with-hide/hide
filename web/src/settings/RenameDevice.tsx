import { useState } from "react";
import { Button } from "../components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "../components/ui/dialog";
import { Input } from "../components/ui/input";
import { useInterfaceTranslation } from "../i18n/client";

/** The most bytes a device's name takes: what hide-core's `device_command::MAX_NAME_BYTES` holds, for a device and a paired machine alike. */
export const MAX_DEVICE_NAME_BYTES = 64;

/** The name a rename sends, trimmed; null when it is empty, too long or more than one line, which the core would refuse. */
export function deviceNameToSave(text: string): string | null {
  const name = text.trim();
  if (name.length === 0 || new TextEncoder().encode(name).length > MAX_DEVICE_NAME_BYTES || /\p{Cc}/u.test(name)) return null;
  return name;
}

/**
 * ⋯ › Rename (operator review 2026-11-23): the name this computer shows a
 * device or a paired machine under, on the rail, in its tabs and in
 * Settings. Save sends one event and closes; the row shows the name the
 * core or the pairing then holds.
 */
export function RenameDevice({ current, onSave, onClose, id }: { current: string; onSave: (name: string) => void; onClose: () => void; id: string }) {
  const { t } = useInterfaceTranslation();
  const [text, setText] = useState(current);
  const name = deviceNameToSave(text);
  const save = name !== null && name !== current;
  return (
    <Dialog open onOpenChange={(next) => { if (!next) onClose(); }}>
      <DialogContent data-device-rename-dialog={id}>
        <form
          className="contents"
          onSubmit={(event) => {
            event.preventDefault();
            if (!save) return;
            onSave(name);
            onClose();
          }}
        >
          <DialogHeader>
            <DialogTitle>{t("devices.renameTitle", { name: current })}</DialogTitle>
            <DialogDescription>{t("devices.renameDescription")}</DialogDescription>
          </DialogHeader>
          <DialogBody>
            <Input autoFocus aria-label={t("devices.renameLabel")} value={text} onChange={(event) => setText(event.target.value)} data-device-rename-input={id} />
          </DialogBody>
          <DialogFooter>
            <Button type="button" variant="secondary" onClick={onClose}>{t("common.cancel")}</Button>
            <Button type="submit" disabled={!save} data-device-rename-go={id}>{t("devices.renameSave")}</Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
