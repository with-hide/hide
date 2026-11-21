// The words for why a task (a worktree, a tab, an agent start, a checkout's
// purpose) failed, for the codes the core sends in `task_operation.failure`
// that no other catalog words (docs/LOCALIZATION.md, Translation boundary).
// `coreError.ts` maps each code to its key; the core's own sentence stays in
// the `task_operation.failed` diagnostic. A purpose failure is followed on
// screen by `workspace.purposeFailure`'s "your text is kept" sentence.
import type { Catalogs } from "../schema";

export const taskReasonsEnglish = {
  "taskReason.workerUnavailable": "Hide could not start the work. Nothing was started; try again.",
  "taskReason.settingsNotReady": "Settings are still loading. Try again.",
  "taskReason.folderMissing": "The working folder no longer exists.",
  "taskReason.purpose.gitFailedSaved": "It was saved to the workspace, but Git did not record it.",
  "taskReason.purpose.gitFailed": "Git did not record it.",
  "taskReason.purpose.tokenFailed": "The terminal engine did not answer.",
  "taskReason.purpose.unknownCheckout": "This checkout is no longer available.",
  "taskReason.purpose.invalid": "A purpose is one line of 80 characters or fewer.",
  "taskReason.purpose.noTarget": "This detached checkout has no workspace to keep a purpose.",
} as const;

const ko = {
  "taskReason.workerUnavailable": "작업을 시작하지 못했어요. 아무것도 시작하지 않았으니 다시 시도하세요.",
  "taskReason.settingsNotReady": "설정을 아직 불러오는 중이에요. 다시 시도하세요.",
  "taskReason.folderMissing": "작업 폴더가 더 이상 없어요.",
  "taskReason.purpose.gitFailedSaved": "작업공간에는 저장했지만 Git에 기록하지 못했어요.",
  "taskReason.purpose.gitFailed": "Git에 기록하지 못했어요.",
  "taskReason.purpose.tokenFailed": "터미널 엔진이 답하지 않았어요.",
  "taskReason.purpose.unknownCheckout": "이 체크아웃을 더 이상 쓸 수 없어요.",
  "taskReason.purpose.invalid": "용도는 80자 이하 한 줄이어야 해요.",
  "taskReason.purpose.noTarget": "이 분리된 체크아웃에는 용도를 저장할 작업공간이 없어요.",
} satisfies Record<keyof typeof taskReasonsEnglish, string>;

const zhCN = {
  "taskReason.workerUnavailable": "Hide 无法开始这项工作。未启动任何内容，请重试。",
  "taskReason.settingsNotReady": "设置仍在加载。请重试。",
  "taskReason.folderMissing": "工作文件夹已不存在。",
  "taskReason.purpose.gitFailedSaved": "已保存到工作区，但 Git 未记录。",
  "taskReason.purpose.gitFailed": "Git 未记录。",
  "taskReason.purpose.tokenFailed": "终端引擎没有响应。",
  "taskReason.purpose.unknownCheckout": "此检出目录已不可用。",
  "taskReason.purpose.invalid": "用途须为一行，且不超过 80 个字符。",
  "taskReason.purpose.noTarget": "此分离的检出目录没有可保存用途的工作区。",
} satisfies Record<keyof typeof taskReasonsEnglish, string>;

const ja = {
  "taskReason.workerUnavailable": "作業を開始できませんでした。何も開始していないので、もう一度お試しください。",
  "taskReason.settingsNotReady": "設定をまだ読み込んでいます。もう一度お試しください。",
  "taskReason.folderMissing": "作業フォルダがもう存在しません。",
  "taskReason.purpose.gitFailedSaved": "ワークスペースには保存しましたが、Git に記録されませんでした。",
  "taskReason.purpose.gitFailed": "Git に記録されませんでした。",
  "taskReason.purpose.tokenFailed": "ターミナルエンジンが応答しませんでした。",
  "taskReason.purpose.unknownCheckout": "このチェックアウトはもう使えません。",
  "taskReason.purpose.invalid": "用途は 80 文字以内の 1 行です。",
  "taskReason.purpose.noTarget": "この切り離されたチェックアウトには、用途を保存するワークスペースがありません。",
} satisfies Record<keyof typeof taskReasonsEnglish, string>;

export const taskReasonsCatalogs = { en: taskReasonsEnglish, ko, "zh-CN": zhCN, ja } satisfies Catalogs<typeof taskReasonsEnglish>;
