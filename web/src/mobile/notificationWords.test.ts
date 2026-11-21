import { readFileSync } from "node:fs";
import { afterEach, describe, expect, it } from "vitest";
import { INTERFACE_LANGUAGES } from "../i18n/locale";
import { clientI18n, translate } from "../i18n/translator";

// The notification extension has no web view, so it words a decrypted notice from a table of its own
// (`NoticeWords.swift`). This holds each entry of that table equal to the catalog entry the web app's service worker
// words the same notice from, in every language, so a change of wording cannot reach one place and not the other.

const source = readFileSync(new URL("../../../mobile/ios/App/HidePushCore/Sources/HidePushCore/NoticeWords.swift", import.meta.url), "utf8");

function swiftTables(): Record<string, Record<string, string>> {
  const tables: Record<string, Record<string, string>> = {};
  for (const [, language, body] of source.matchAll(/"([a-z]{2})": \[\n([\s\S]*?)\n {8}\],/g)) {
    const entries: Record<string, string> = {};
    for (const [, key, value] of body!.matchAll(/"([a-z_]+)": "((?:[^"\\]|\\.)*)"/g)) entries[key!] = value!;
    tables[language!] = entries;
  }
  return tables;
}

const STATUS = ["error", "question", "approval", "working", "done", "idle", "unknown", "waiting", "unfinished", "stopped"] as const;

function catalog(): Record<string, string> {
  return {
    needs_you: translate("mobile.group.needs_you"),
    done: translate("mobile.group.done"),
    observer_unconfirmed: translate("mobile.notice.observerUnconfirmed"),
    letter_undelivered: translate("mobile.notice.letterUndelivered"),
    ...Object.fromEntries(STATUS.map((code) => [`status_${code}`, translate(`agents.status.${code}`)])),
    status_look: translate("agents.mark.look"),
  };
}

afterEach(async () => { await clientI18n.changeLanguage("en"); });

describe("the notification extension's words", () => {
  const tables = swiftTables();

  it("has a table for each language the app speaks", () => {
    expect(Object.keys(tables).sort()).toEqual(["en", "ja", "ko", "zh"]);
  });

  for (const language of INTERFACE_LANGUAGES) {
    // The table is keyed by the language's first part, as the phone's preferred languages are read (`zh-Hans-CN` is `zh`).
    const code = language.split("-")[0]!;
    it(`says what the catalog says in ${language}`, async () => {
      await clientI18n.changeLanguage(language);
      const wanted = catalog();
      const table = tables[code]!;
      for (const [key, word] of Object.entries(wanted)) expect(table[key], `${language} ${key}`).toBe(word);
      // Nothing in the table but the catalog's words.
      expect(Object.keys(table).filter((key) => !(key in wanted))).toEqual([]);
    });
  }
});
