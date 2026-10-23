import type { TFunction } from "i18next";
import { diffReasonsEnglish } from "./i18n/resources/diffReasons";
import type { DiffNotice } from "./snapshot";

// The words for why a diff is short or absent; a code this build does not
// know reads as a diff Git could not show.

export function diffNoticeText(notice: DiffNotice, t: TFunction<"translation">): string {
  switch (notice.code) {
    case "truncated":
      return t("diffReason.truncated", { limit: notice.limit_kb ?? 0 });
    case "outside":
      return t("diffReason.outside", { root: notice.root ?? "" });
    default: {
      const key = `diffReason.${notice.code}`;
      return key in diffReasonsEnglish ? t(key as keyof typeof diffReasonsEnglish) : t("diffReason.unreadable");
    }
  }
}
