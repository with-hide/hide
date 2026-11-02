import { useInterfaceTranslation } from "../i18n/translator";
import { hostedPhone } from "./relayRuntime";
import { noticeText, UNREACHABLE_NOTICE } from "./protocol";
import { usePhone } from "./store";

export function OfflineText() {
  const { t } = useInterfaceTranslation();
  const name = usePhone((s) => s.macName);
  const error = usePhone((s) => s.relayError);
  return hostedPhone() ? t(error === "busy" ? "mobile.relay.busy" : "mobile.relay.offline", { name }) : noticeText(t, UNREACHABLE_NOTICE);
}
