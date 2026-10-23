// The words for why a History or View diff is short or absent. The core and
// the repository's host send a code (`DiffNotice` in `hide-node-link`), and
// Git's own reason for a diff it could not take stays in the core's
// `diff_unreadable` diagnostic (docs/LOCALIZATION.md, Translation boundary).
import type { Catalogs } from "../schema";

export const diffReasonsEnglish = {
  "diffReason.truncated": "This diff is larger than {{limit}} KB and is shown cut short.",
  "diffReason.no_base": "This checkout has no base branch to compare with.",
  "diffReason.no_branch_changes": "This file has no changes on this branch.",
  "diffReason.no_uncommitted_changes": "This file has no uncommitted changes.",
  "diffReason.unreadable": "Git could not show this diff.",
  "diffReason.outside": "This file is outside {{root}}, the folder History reads for the project in front. Its diff shows when a project that holds it is in front.",
} as const;

const ko = {
  "diffReason.truncated": "변경 내용이 {{limit}} KB보다 커서 앞부분만 보여요.",
  "diffReason.no_base": "이 체크아웃에는 비교할 기준 브랜치가 없어요.",
  "diffReason.no_branch_changes": "이 브랜치에서 이 파일은 바뀐 내용이 없어요.",
  "diffReason.no_uncommitted_changes": "이 파일에는 커밋하지 않은 변경이 없어요.",
  "diffReason.unreadable": "Git이 이 변경 내용을 보여 주지 못했어요.",
  "diffReason.outside": "이 파일은 앞에 있는 프로젝트의 변경 기록 폴더 바깥에 있어요: {{root}}. 이 파일이 든 프로젝트를 앞으로 가져오면 변경 내용이 보여요.",
} satisfies Record<keyof typeof diffReasonsEnglish, string>;

const zhCN = {
  "diffReason.truncated": "此差异超过 {{limit}} KB，仅显示前面部分。",
  "diffReason.no_base": "此检出目录没有可比较的基准分支。",
  "diffReason.no_branch_changes": "此文件在这个分支上没有改动。",
  "diffReason.no_uncommitted_changes": "此文件没有未提交的改动。",
  "diffReason.unreadable": "Git 无法显示此差异。",
  "diffReason.outside": "此文件位于 {{root}} 之外，这是当前项目的历史记录所读取的文件夹。把包含它的项目切到前面后会显示其差异。",
} satisfies Record<keyof typeof diffReasonsEnglish, string>;

const ja = {
  "diffReason.truncated": "この差分は {{limit}} KB を超えるため、先頭部分だけを表示しています。",
  "diffReason.no_base": "このチェックアウトには比較するベースブランチがありません。",
  "diffReason.no_branch_changes": "このファイルはこのブランチで変更されていません。",
  "diffReason.no_uncommitted_changes": "このファイルには未コミットの変更がありません。",
  "diffReason.unreadable": "Git がこの差分を表示できませんでした。",
  "diffReason.outside": "このファイルは {{root}} の外にあります。ここは手前のプロジェクトの履歴が読むフォルダです。このファイルを含むプロジェクトを手前にすると差分が表示されます。",
} satisfies Record<keyof typeof diffReasonsEnglish, string>;

export const diffReasonsCatalogs = { en: diffReasonsEnglish, ko, "zh-CN": zhCN, ja } satisfies Catalogs<typeof diffReasonsEnglish>;
