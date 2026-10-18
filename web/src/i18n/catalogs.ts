import { groupTabsCatalogs, groupTabsEnglish } from "./resources/groupTabs";
import { menusCatalogs, menusEnglish } from "./resources/menus";
import { projectSettingsCatalogs, projectSettingsEnglish } from "./resources/projectSettings";
import { usageCatalogs, usageEnglish } from "./resources/usage";
import { commandsCatalogs, commandsEnglish } from "./resources/commands";
import { agentSessionsCatalogs, agentSessionsEnglish } from "./resources/agentSessions";
import { taskLensCatalogs, taskLensEnglish } from "./resources/taskLens";
import { archiveCatalogs, archiveEnglish } from "./resources/archive";
import { boardCatalogs, boardEnglish } from "./resources/board";
import { cleanupCatalogs, cleanupEnglish } from "./resources/cleanup";
import { commonCatalogs, commonEnglish } from "./resources/common";
import { agentPresentationCatalogs, agentPresentationEnglish } from "./resources/agentPresentation";
import { devicesCatalogs, devicesEnglish } from "./resources/devices";
import { agentsCatalogs, agentsEnglish } from "./resources/agents";
import { hideAiCatalogs, hideAiEnglish } from "./resources/hideAi";
import { issueSettingsCatalogs, issueSettingsEnglish } from "./resources/issueSettings";
import { historyCatalogs, historyEnglish } from "./resources/history";
import { explorerCatalogs, explorerEnglish } from "./resources/explorer";
import { panesCatalogs, panesEnglish } from "./resources/panes";
import { sessionsCatalogs, sessionsEnglish } from "./resources/sessions";
import { searchCatalogs, searchEnglish } from "./resources/search";
import { documentsCatalogs, documentsEnglish } from "./resources/documents";
import { editorSearchCatalogs, editorSearchEnglish } from "./resources/editorSearch";
import { shellCatalogs, shellEnglish } from "./resources/shell";
import { issuesCatalogs, issuesEnglish } from "./resources/issues";
import { mobileCatalogs, mobileEnglish } from "./resources/mobile";
import { mobileSetupCatalogs, mobileSetupEnglish } from "./resources/mobileSetup";
import { coreMoveCatalogs, coreMoveEnglish } from "./resources/coreMove";
import { nativeCatalogs, nativeEnglish } from "./resources/native";
import { overviewCatalogs, overviewEnglish } from "./resources/overview";
import { factoryCatalogs, factoryEnglish } from "./resources/factory";
import { firstRunCatalogs, firstRunEnglish } from "./resources/firstRun";
import { memoryCatalogs, memoryEnglish } from "./resources/memory";
import { prWorkCatalogs, prWorkEnglish } from "./resources/prWork";
import { prListCatalogs, prListEnglish } from "./resources/prList";
import { workCatalogs, workEnglish } from "./resources/work";
import { linksCatalogs, linksEnglish } from "./resources/links";
import { feedbackCatalogs, feedbackEnglish } from "./resources/feedback";
import { requestsCatalogs, requestsEnglish } from "./resources/requests";
import { settingsCatalogs, settingsEnglish } from "./resources/settings";
import { workspaceCatalogs, workspaceEnglish } from "./resources/workspace";
import type { Catalogs } from "./schema";
import { sessionWorkCatalogs, sessionWorkEnglish } from "./resources/sessionWork";
import { deviceOverviewCatalogs, deviceOverviewEnglish } from "./resources/deviceOverview";

export const english = {
  ...groupTabsEnglish,
  ...menusEnglish,
  ...sessionWorkEnglish,
  ...deviceOverviewEnglish,
  ...agentSessionsEnglish,
  ...taskLensEnglish,
  ...archiveEnglish,
  ...shellEnglish,
  ...editorSearchEnglish,
  ...documentsEnglish,
  ...searchEnglish,
  ...panesEnglish,
  ...explorerEnglish,
  ...historyEnglish,
  ...sessionsEnglish,
  ...agentPresentationEnglish,
  ...commonEnglish,
  ...commandsEnglish,
  ...nativeEnglish,
  ...mobileEnglish,
  ...settingsEnglish,
  ...usageEnglish,
  ...cleanupEnglish,
  ...devicesEnglish,
  ...mobileSetupEnglish,
  ...coreMoveEnglish,
  ...workspaceEnglish,
  ...issueSettingsEnglish,
  ...hideAiEnglish,
  ...agentsEnglish,
  ...requestsEnglish,
  ...issuesEnglish,
  ...prWorkEnglish,
  ...boardEnglish,
  ...prListEnglish,
  ...workEnglish,
  ...linksEnglish,
  ...feedbackEnglish,
  ...overviewEnglish,
  ...factoryEnglish,
  ...firstRunEnglish,
  ...memoryEnglish,
  ...projectSettingsEnglish,
} as const;

export type MessageKey = keyof typeof english;

export const catalogs = {
  en: english,
  ko: {
    ...groupTabsCatalogs.ko,
    ...menusCatalogs.ko,
    ...sessionWorkCatalogs.ko,
    ...deviceOverviewCatalogs.ko,
    ...agentSessionsCatalogs.ko,
    ...taskLensCatalogs.ko,
    ...archiveCatalogs.ko,
    ...shellCatalogs.ko,
    ...editorSearchCatalogs.ko,
    ...documentsCatalogs.ko,
    ...searchCatalogs.ko,
    ...panesCatalogs.ko,
    ...explorerCatalogs.ko,
    ...historyCatalogs.ko,
    ...sessionsCatalogs.ko,
    ...agentPresentationCatalogs.ko,
    ...commonCatalogs.ko,
    ...commandsCatalogs.ko,
    ...nativeCatalogs.ko,
    ...mobileCatalogs.ko,
    ...settingsCatalogs.ko,
    ...usageCatalogs.ko,
    ...cleanupCatalogs.ko,
    ...devicesCatalogs.ko,
    ...mobileSetupCatalogs.ko,
    ...coreMoveCatalogs.ko,
    ...workspaceCatalogs.ko,
    ...issueSettingsCatalogs.ko,
    ...hideAiCatalogs.ko,
    ...agentsCatalogs.ko,
    ...requestsCatalogs.ko,
    ...issuesCatalogs.ko,
    ...prWorkCatalogs.ko,
    ...boardCatalogs.ko,
    ...prListCatalogs.ko,
    ...workCatalogs.ko,
    ...linksCatalogs.ko,
    ...feedbackCatalogs.ko,
    ...overviewCatalogs.ko,
    ...factoryCatalogs.ko,
    ...firstRunCatalogs.ko,
    ...memoryCatalogs.ko,
    ...projectSettingsCatalogs.ko,
  },
  "zh-CN": {
    ...groupTabsCatalogs["zh-CN"],
    ...menusCatalogs["zh-CN"],
    ...sessionWorkCatalogs["zh-CN"],
    ...deviceOverviewCatalogs["zh-CN"],
    ...agentSessionsCatalogs["zh-CN"],
    ...taskLensCatalogs["zh-CN"],
    ...archiveCatalogs["zh-CN"],
    ...shellCatalogs["zh-CN"],
    ...editorSearchCatalogs["zh-CN"],
    ...documentsCatalogs["zh-CN"],
    ...searchCatalogs["zh-CN"],
    ...panesCatalogs["zh-CN"],
    ...explorerCatalogs["zh-CN"],
    ...historyCatalogs["zh-CN"],
    ...sessionsCatalogs["zh-CN"],
    ...agentPresentationCatalogs["zh-CN"],
    ...commonCatalogs["zh-CN"],
    ...commandsCatalogs["zh-CN"],
    ...nativeCatalogs["zh-CN"],
    ...mobileCatalogs["zh-CN"],
    ...settingsCatalogs["zh-CN"],
    ...usageCatalogs["zh-CN"],
    ...cleanupCatalogs["zh-CN"],
    ...devicesCatalogs["zh-CN"],
    ...mobileSetupCatalogs["zh-CN"],
    ...coreMoveCatalogs["zh-CN"],
    ...workspaceCatalogs["zh-CN"],
    ...issueSettingsCatalogs["zh-CN"],
    ...hideAiCatalogs["zh-CN"],
    ...agentsCatalogs["zh-CN"],
    ...requestsCatalogs["zh-CN"],
    ...issuesCatalogs["zh-CN"],
    ...prWorkCatalogs["zh-CN"],
    ...boardCatalogs["zh-CN"],
    ...prListCatalogs["zh-CN"],
    ...workCatalogs["zh-CN"],
    ...linksCatalogs["zh-CN"],
    ...feedbackCatalogs["zh-CN"],
    ...overviewCatalogs["zh-CN"],
    ...factoryCatalogs["zh-CN"],
    ...firstRunCatalogs["zh-CN"],
    ...memoryCatalogs["zh-CN"],
    ...projectSettingsCatalogs["zh-CN"],
  },
  ja: {
    ...groupTabsCatalogs.ja,
    ...menusCatalogs.ja,
    ...sessionWorkCatalogs.ja,
    ...deviceOverviewCatalogs.ja,
    ...agentSessionsCatalogs.ja,
    ...taskLensCatalogs.ja,
    ...archiveCatalogs.ja,
    ...shellCatalogs.ja,
    ...editorSearchCatalogs.ja,
    ...documentsCatalogs.ja,
    ...searchCatalogs.ja,
    ...panesCatalogs.ja,
    ...explorerCatalogs.ja,
    ...historyCatalogs.ja,
    ...sessionsCatalogs.ja,
    ...agentPresentationCatalogs.ja,
    ...commonCatalogs.ja,
    ...commandsCatalogs.ja,
    ...nativeCatalogs.ja,
    ...mobileCatalogs.ja,
    ...settingsCatalogs.ja,
    ...usageCatalogs.ja,
    ...cleanupCatalogs.ja,
    ...devicesCatalogs.ja,
    ...mobileSetupCatalogs.ja,
    ...coreMoveCatalogs.ja,
    ...workspaceCatalogs.ja,
    ...issueSettingsCatalogs.ja,
    ...hideAiCatalogs.ja,
    ...agentsCatalogs.ja,
    ...requestsCatalogs.ja,
    ...issuesCatalogs.ja,
    ...prWorkCatalogs.ja,
    ...boardCatalogs.ja,
    ...prListCatalogs.ja,
    ...workCatalogs.ja,
    ...linksCatalogs.ja,
    ...feedbackCatalogs.ja,
    ...overviewCatalogs.ja,
    ...factoryCatalogs.ja,
    ...firstRunCatalogs.ja,
    ...memoryCatalogs.ja,
    ...projectSettingsCatalogs.ja,
  },
} satisfies Catalogs<typeof english>;
