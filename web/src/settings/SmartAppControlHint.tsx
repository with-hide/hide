import { useInterfaceTranslation } from "../i18n/client";
import type { Device } from "../snapshot";

/** The site's section on turning Smart App Control off, which a Windows device whose Hide did not start links to. */
export const SMART_APP_CONTROL_DOCS = "https://withhide.dev/docs/reference/troubleshooting/#smart-app-control-blocks-hide-on-windows-11";

/**
 * Whether Hide's program was started on the device's Windows and never
 * answered: its connection's helper (`windows_not_started`), or the Hide a
 * switch started there. Smart App Control blocks an unsigned program that
 * way, and Windows says so only on that PC's screen, so the row says where
 * to look; it cannot tell that it was the block (issue #2855).
 */
export function startedSilentOnWindows(device: Pick<Device, "problem" | "switch" | "host">): boolean {
  if (device.problem === "windows_not_started") return true;
  return device.switch?.state === "failed" && device.switch.reason === "ssh_core_not_started" && /^windows/i.test(device.host?.platform ?? "");
}

/** "if Smart App Control is on, how to turn it off", the last words linking to that section. */
export function SmartAppControlHint({ deviceId }: { deviceId: string }) {
  const { t } = useInterfaceTranslation();
  return (
    <>
      {t("devices.sac.if")}{" "}
      <a href={SMART_APP_CONTROL_DOCS} target="_blank" rel="noopener noreferrer" className="text-primary underline underline-offset-2 outline-none focus-visible:ring-1 focus-visible:ring-ring" data-device-sac-docs={deviceId}>
        {t("devices.sac.howTo")}
      </a>
    </>
  );
}
