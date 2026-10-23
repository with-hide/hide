import { describe, expect, it } from "vitest";
import { CHAPTER_WORDS } from "../practice/words";
import { catalogs, english } from "./catalogs";
import { commandsEnglish } from "./resources/commands";
import { agentSessionsEnglish } from "./resources/agentSessions";
import { memoryEnglish } from "./resources/memory";
import { boardEnglish } from "./resources/board";
import { cleanupEnglish } from "./resources/cleanup";
import { commonEnglish } from "./resources/common";
import { agentPresentationEnglish } from "./resources/agentPresentation";
import { agentsEnglish } from "./resources/agents";
import { devicesEnglish } from "./resources/devices";
import { hideAiEnglish } from "./resources/hideAi";
import { issueSettingsEnglish } from "./resources/issueSettings";
import { historyEnglish } from "./resources/history";
import { explorerEnglish } from "./resources/explorer";
import { panesEnglish } from "./resources/panes";
import { sessionsEnglish } from "./resources/sessions";
import { searchEnglish } from "./resources/search";
import { documentsEnglish } from "./resources/documents";
import { feedbackEnglish } from "./resources/feedback";
import { editorSearchEnglish } from "./resources/editorSearch";
import { shellEnglish } from "./resources/shell";
import { issuesEnglish } from "./resources/issues";
import { mobileEnglish } from "./resources/mobile";
import { mobileSetupEnglish } from "./resources/mobileSetup";
import { nativeEnglish } from "./resources/native";
import { overviewEnglish } from "./resources/overview";
import { prWorkEnglish } from "./resources/prWork";
import { prListEnglish } from "./resources/prList";
import { requestsEnglish } from "./resources/requests";
import { settingsEnglish } from "./resources/settings";
import { workspaceEnglish } from "./resources/workspace";
import { validateCatalogs } from "./schema";

describe("interface resources", () => {
  it("provides every key and insertion in all four languages", () => {
    expect(() => validateCatalogs(english, catalogs)).not.toThrow();
  });

  it("keeps the practice chapters' words apart from every catalog key", () => {
    expect(Object.keys(CHAPTER_WORDS.en).filter((key) => key in english)).toEqual([]);
  });

  it("gives every agent status word its own text in each language", () => {
    for (const [locale, catalog] of Object.entries(catalogs)) {
      const words = Object.entries(catalog as Record<string, string>)
        .filter(([key]) => /^agents\.status\.(error|question|approval|working|done|idle|unknown|waiting|stopped)$/.test(key))
        .map(([, word]) => word);
      expect(words.length, locale).toBe(9);
      expect(new Set(words).size, locale).toBe(words.length);
    }
  });

  it("keeps domain keys separate so composition cannot overwrite a message", () => {
    const keys = [
      agentSessionsEnglish,
    shellEnglish,
    editorSearchEnglish,
      documentsEnglish,
      feedbackEnglish,
      searchEnglish,
      panesEnglish,
      explorerEnglish,
      historyEnglish,
      sessionsEnglish,
      agentPresentationEnglish,
      commonEnglish,
      commandsEnglish,
      nativeEnglish,
      mobileEnglish,
      settingsEnglish,
      cleanupEnglish,
      devicesEnglish,
      mobileSetupEnglish,
      workspaceEnglish,
      issueSettingsEnglish,
      hideAiEnglish,
      agentsEnglish,
      requestsEnglish,
      issuesEnglish,
      prWorkEnglish,
      boardEnglish,
      prListEnglish,
      overviewEnglish,
      memoryEnglish,
    ].flatMap((schema) => Object.keys(schema));
    expect(new Set(keys).size).toBe(keys.length);
  });
});
