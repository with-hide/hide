import { groupTabsCatalogs, groupTabsEnglish } from "./resources/groupTabs";
import { overviewBacklogCatalogs, overviewBacklogEnglish } from "../overviewBacklogMessages";
import { overviewUsageCatalogs, overviewUsageEnglish } from "../overviewUsageMessages";
import { overviewHistoryCatalogs, overviewHistoryEnglish } from "../overviewHistoryMessages";
import { overviewGraphCatalogs, overviewGraphEnglish } from "../overviewGraphMessages";
import { menusCatalogs, menusEnglish } from "./resources/menus";
import { projectSettingsCatalogs, projectSettingsEnglish } from "./resources/projectSettings";
import { projectToolCatalogs, projectToolEnglish } from "./resources/projectTool";
import { usageCatalogs, usageEnglish } from "./resources/usage";
import { commandsCatalogs, commandsEnglish } from "./resources/commands";
import { agentSessionsCatalogs, agentSessionsEnglish } from "./resources/agentSessions";
import { taskLensCatalogs, taskLensEnglish } from "./resources/taskLens";
import { missionsCatalogs, missionsEnglish } from "./resources/missions";
import { plansCatalogs, plansEnglish } from "./resources/plans";
import { archiveCatalogs, archiveEnglish } from "./resources/archive";
import { boardCatalogs, boardEnglish } from "./resources/board";
import { cleanupCatalogs, cleanupEnglish } from "./resources/cleanup";
import { commonCatalogs, commonEnglish } from "./resources/common";
import { agentPresentationCatalogs, agentPresentationEnglish } from "./resources/agentPresentation";
import { devicesCatalogs, devicesEnglish } from "./resources/devices";
import { kitReasonsCatalogs, kitReasonsEnglish } from "./resources/kitReasons";
import { issueReasonsCatalogs, issueReasonsEnglish } from "./resources/issueReasons";
import { agentActionReasonsCatalogs, agentActionReasonsEnglish } from "./resources/agentActionReasons";
import { fileReasonsCatalogs, fileReasonsEnglish } from "./resources/fileReasons";
import { taskReasonsCatalogs, taskReasonsEnglish } from "./resources/taskReasons";
import { deletionGateCatalogs, deletionGateEnglish } from "./resources/deletionGate";
import { githubReasonsCatalogs, githubReasonsEnglish } from "./resources/githubReasons";
import { removalReasonsCatalogs, removalReasonsEnglish } from "./resources/removalReasons";
import { diffReasonsCatalogs, diffReasonsEnglish } from "./resources/diffReasons";
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
import { firstRunCatalogs, firstRunEnglish } from "./resources/firstRun";
import { herdrTakeoverCatalogs, herdrTakeoverEnglish } from "./resources/herdrTakeover";
import { memoryCatalogs, memoryEnglish } from "./resources/memory";
import { prWorkCatalogs, prWorkEnglish } from "./resources/prWork";
import { prListCatalogs, prListEnglish } from "./resources/prList";
import { workCatalogs, workEnglish } from "./resources/work";
import { linksCatalogs, linksEnglish } from "./resources/links";
import { feedbackCatalogs, feedbackEnglish } from "./resources/feedback";
import { requestsCatalogs, requestsEnglish } from "./resources/requests";
import { settingsCatalogs, settingsEnglish } from "./resources/settings";
import { workspaceCatalogs, workspaceEnglish } from "./resources/workspace";
import { checkoutGitCatalogs, checkoutGitEnglish } from "./resources/checkoutGit";
import type { Catalogs } from "./schema";
import { coreErrorsCatalogs, coreErrorsEnglish } from "./resources/coreErrors";
import { sessionWorkCatalogs, sessionWorkEnglish } from "./resources/sessionWork";
import { deviceOverviewCatalogs, deviceOverviewEnglish } from "./resources/deviceOverview";
import { overviewRedesignCatalogs, overviewRedesignEnglish } from "./resources/overviewRedesign";

export const english = {
  ...groupTabsEnglish,
  ...menusEnglish,
  ...sessionWorkEnglish,
  ...deviceOverviewEnglish,
  ...overviewRedesignEnglish,
  ...overviewBacklogEnglish,
  ...overviewUsageEnglish,
  ...overviewHistoryEnglish,
  ...overviewGraphEnglish,
  ...agentSessionsEnglish,
  ...taskLensEnglish,
  ...missionsEnglish,
  ...plansEnglish,
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
  ...kitReasonsEnglish,
  ...issueReasonsEnglish,
  ...agentActionReasonsEnglish,
  ...fileReasonsEnglish,
  ...taskReasonsEnglish,
  ...deletionGateEnglish,
  ...githubReasonsEnglish,
  ...removalReasonsEnglish,
  ...diffReasonsEnglish,
  ...mobileSetupEnglish,
  ...coreMoveEnglish,
  ...workspaceEnglish,
  ...checkoutGitEnglish,
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
  ...firstRunEnglish,
  ...herdrTakeoverEnglish,
  ...memoryEnglish,
  ...projectSettingsEnglish,
  ...projectToolEnglish,
  ...coreErrorsEnglish,
} as const;

export type MessageKey = keyof typeof english;

export const catalogs = {
  en: english,
  ko: {
    ...groupTabsCatalogs.ko,
    ...menusCatalogs.ko,
    ...sessionWorkCatalogs.ko,
    ...deviceOverviewCatalogs.ko,
    ...overviewRedesignCatalogs.ko,
    ...overviewBacklogCatalogs.ko,
    ...overviewUsageCatalogs.ko,
    ...overviewHistoryCatalogs.ko,
    ...overviewGraphCatalogs.ko,
    ...agentSessionsCatalogs.ko,
    ...taskLensCatalogs.ko,
    ...missionsCatalogs.ko,
    ...plansCatalogs.ko,
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
    ...kitReasonsCatalogs.ko,
    ...issueReasonsCatalogs.ko,
    ...agentActionReasonsCatalogs.ko,
    ...fileReasonsCatalogs.ko,
    ...taskReasonsCatalogs.ko,
    ...deletionGateCatalogs.ko,
    ...githubReasonsCatalogs.ko,
    ...removalReasonsCatalogs.ko,
    ...diffReasonsCatalogs.ko,
    ...mobileSetupCatalogs.ko,
    ...coreMoveCatalogs.ko,
    ...workspaceCatalogs.ko,
    ...checkoutGitCatalogs.ko,
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
    ...firstRunCatalogs.ko,
    ...herdrTakeoverCatalogs.ko,
    ...memoryCatalogs.ko,
    ...projectSettingsCatalogs.ko,
    ...projectToolCatalogs.ko,
    ...coreErrorsCatalogs.ko,
  },
  "zh-CN": {
    ...groupTabsCatalogs["zh-CN"],
    ...menusCatalogs["zh-CN"],
    ...sessionWorkCatalogs["zh-CN"],
    ...deviceOverviewCatalogs["zh-CN"],
    ...overviewRedesignCatalogs["zh-CN"],
    ...overviewBacklogCatalogs["zh-CN"],
    ...overviewUsageCatalogs["zh-CN"],
    ...overviewHistoryCatalogs["zh-CN"],
    ...overviewGraphCatalogs["zh-CN"],
    ...agentSessionsCatalogs["zh-CN"],
    ...taskLensCatalogs["zh-CN"],
    ...missionsCatalogs["zh-CN"],
    ...plansCatalogs["zh-CN"],
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
    ...kitReasonsCatalogs["zh-CN"],
    ...issueReasonsCatalogs["zh-CN"],
    ...agentActionReasonsCatalogs["zh-CN"],
    ...fileReasonsCatalogs["zh-CN"],
    ...taskReasonsCatalogs["zh-CN"],
    ...deletionGateCatalogs["zh-CN"],
    ...githubReasonsCatalogs["zh-CN"],
    ...removalReasonsCatalogs["zh-CN"],
    ...diffReasonsCatalogs["zh-CN"],
    ...mobileSetupCatalogs["zh-CN"],
    ...coreMoveCatalogs["zh-CN"],
    ...workspaceCatalogs["zh-CN"],
    ...checkoutGitCatalogs["zh-CN"],
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
    ...firstRunCatalogs["zh-CN"],
    ...herdrTakeoverCatalogs["zh-CN"],
    ...memoryCatalogs["zh-CN"],
    ...projectSettingsCatalogs["zh-CN"],
    ...projectToolCatalogs["zh-CN"],
    ...coreErrorsCatalogs["zh-CN"],
  },
  ja: {
    ...groupTabsCatalogs.ja,
    ...menusCatalogs.ja,
    ...sessionWorkCatalogs.ja,
    ...deviceOverviewCatalogs.ja,
    ...overviewRedesignCatalogs.ja,
    ...overviewBacklogCatalogs.ja,
    ...overviewUsageCatalogs.ja,
    ...overviewHistoryCatalogs.ja,
    ...overviewGraphCatalogs.ja,
    ...agentSessionsCatalogs.ja,
    ...taskLensCatalogs.ja,
    ...missionsCatalogs.ja,
    ...plansCatalogs.ja,
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
    ...kitReasonsCatalogs.ja,
    ...issueReasonsCatalogs.ja,
    ...agentActionReasonsCatalogs.ja,
    ...fileReasonsCatalogs.ja,
    ...taskReasonsCatalogs.ja,
    ...deletionGateCatalogs.ja,
    ...githubReasonsCatalogs.ja,
    ...removalReasonsCatalogs.ja,
    ...diffReasonsCatalogs.ja,
    ...mobileSetupCatalogs.ja,
    ...coreMoveCatalogs.ja,
    ...workspaceCatalogs.ja,
    ...checkoutGitCatalogs.ja,
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
    ...firstRunCatalogs.ja,
    ...herdrTakeoverCatalogs.ja,
    ...memoryCatalogs.ja,
    ...projectSettingsCatalogs.ja,
    ...projectToolCatalogs.ja,
    ...coreErrorsCatalogs.ja,
  },
} satisfies Catalogs<typeof english>;
