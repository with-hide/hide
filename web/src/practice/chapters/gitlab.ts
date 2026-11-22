// Flow 9: a project on gitlab.com. Its merge requests sit in the PR list
// beside GitHub's pull requests, written `!12` with GitLab's mark; the learner
// opens one, reads its card, which names GitLab and draws only what GitLab
// offers, and finds GitLab's row in Settings › Remote repositories.

import { chapter } from "../chapter";
import { GITLAB_WORLD, MR_CARD, MR_ROW, showsMr, showsPrs, type GitlabModel } from "../worlds/gitlab";

export default chapter<GitlabModel>({
  id: "gitlab",
  number: 9,
  version: 1,
  since: "1.3.0",
  world: GITLAB_WORLD,
  quest: true,
  preview: "rows",
  steps: [
    { id: "list", quest: true, target: '[data-sidebar-lens-option="prs"]', side: "right", command: "open_sidebar_prs", gesture: { click: true }, done: showsPrs },
    // Both hosts in one list: `!12` with GitLab's mark, `#41` with GitHub's.
    { id: "mark", target: MR_ROW, side: "right" },
    { id: "open", quest: true, target: MR_ROW, side: "right", gesture: { click: true }, done: showsMr },
    { id: "card", target: `${MR_CARD} [data-pr-card-header]`, side: "left" },
    // The page on gitlab.com, from the address the core carried.
    { id: "page", target: `${MR_CARD} [data-pr-card-github]`, side: "left" },
    { id: "settings", quest: true, target: "[data-sidebar]", side: "right", command: "settings", shown: "[data-settings-tab]" },
    { id: "remotes", quest: true, target: '[data-settings-tab="github"]', side: "right", gesture: { click: true }, shown: '[data-settings-host="gitlab"]' },
    { id: "hosts", target: '[data-settings-group="github"]', side: "left" },
    { id: "end", target: '[data-settings-host="gitlab"]', side: "left" },
  ],
});
