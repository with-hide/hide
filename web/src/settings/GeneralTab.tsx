import type { Actions } from "../actions";
import { Group, Note, Row } from "../components/settings-rows";
import { Switch } from "../components/ui/switch";
import { useInterfaceTranslation } from "../i18n/client";
import { useShellStore } from "../store";
import { AppearanceGroup } from "./AppearanceGroup";

export function GeneralTab({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const enabled = useShellStore((s) => s.rest?.ui_state?.product_analytics_enabled ?? true);
  const connected = useShellStore((s) => s.connection === "live");
  return <>
    <AppearanceGroup actions={actions} />
    <Group title={t("settings.privacy")} data-settings-group="privacy">
      <Row label={t("settings.productAnalytics")} detail={<Note>{t("settings.productAnalyticsDetail")}</Note>}>
        <Switch aria-label={t("settings.productAnalytics")} checked={enabled} disabled={!connected} onCheckedChange={actions.setProductAnalytics} data-product-analytics="true" />
      </Row>
    </Group>
  </>;
}
