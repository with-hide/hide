import { useInterfaceTranslation } from "../i18n/client";

/** The site's section on turning Smart App Control off, which a Windows device whose Hide did not start links to. */
export const SMART_APP_CONTROL_DOCS = "https://withhide.dev/docs/reference/troubleshooting/#smart-app-control-blocks-hide-on-windows-11";

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
