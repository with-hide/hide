// Builds the shell's Intl formatters and plural rules before a test file's tests run.
// On the Windows runner a fresh test worker spent 0.3 to 0.9 s building its first ones,
// and several seconds while the suite's first files loaded, inside whichever test formatted
// a number first, which then passed its 5 s timeout (with-hide/hide-private issue 354).
// Built here, the cost lands once per file before any test, where module loading already is.
import { formatDateTime, formatNumber, formatPercent, formatRelativeTime, formatUnit } from "../src/i18n/format";
import { INTERFACE_LANGUAGES } from "../src/i18n/locale";

for (const language of INTERFACE_LANGUAGES) {
  formatNumber(language, 1);
  formatPercent(language, 1);
  formatRelativeTime(language, -1, "minute");
  for (const unit of ["second", "minute", "hour", "day"] as const) formatUnit(language, 1, unit);
  formatDateTime(language, 0, { dateStyle: "medium", timeStyle: "short" });
  // i18next picks plural forms with Intl.PluralRules.
  new Intl.PluralRules(language).select(1);
}
