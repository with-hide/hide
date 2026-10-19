import type { Actions } from "../actions";
import { Group, Note, Row } from "../components/settings-rows";
import { Switch } from "../components/ui/switch";
import { useInterfaceTranslation } from "../i18n/client";
import { useShellStore } from "../store";

export function PrivacyTab({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const enabled = useShellStore((s) => s.rest?.ui_state?.product_analytics_enabled ?? true);
  const connected = useShellStore((s) => s.connection === "live");
  return (
    <Group data-settings-group="privacy">
      <Row label={t("settings.productAnalytics")} detail={<Note>{t("settings.productAnalyticsDetail")}</Note>}>
        <Switch aria-label={t("settings.productAnalytics")} checked={enabled} disabled={!connected} onCheckedChange={actions.setProductAnalytics} data-product-analytics="true" />
      </Row>
    </Group>
  );
}
