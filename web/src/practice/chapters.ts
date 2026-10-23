// The practice chapters as data: what each step points at, what it says, which
// command performs it, and the world state that means it is done. A step
// names a command, never a chord, so the label and the key follow the host,
// the system and the operator's own bindings the way every other screen does.

import type { CommandId } from "../shortcuts";
import { chapterOneModel, focusedPane, type PracticeModel } from "./practiceWorld";

/** How a step is performed when it has no command: a click or a right-click on its target. */
export type StepGesture = "click" | "contextmenu";

export type PracticeStep = {
  id: string;
  /** The quest card's line. */
  title: string;
  /** The bubble beside the target. */
  say: string;
  /** What the spotlight rings, a selector into the real shell. */
  target: string;
  /** Where the bubble sits beside the target. */
  side: "right" | "left" | "top" | "bottom";
  command?: CommandId;
  gesture?: StepGesture;
  /** A real menu item the step goes on to press, by its visible text, once the gesture has opened the menu. */
  menuItem?: string;
  /** A field the step fills for the learner, so the step stays one press. */
  fill?: { selector: string; value: string };
  done: (model: PracticeModel) => boolean;
};

export type PracticeChapter = {
  id: string;
  number: number;
  title: string;
  /** Bumped when the steps change, so a learner's progress on the old steps reads as changed. */
  version: number;
  intro: string;
  outro: string;
  model: () => PracticeModel;
  steps: PracticeStep[];
};

const tabOf = (model: PracticeModel, id: string) => model.tabs.find((tab) => tab.id === id);
const paneCount = (model: PracticeModel) => {
  const count = (node: PracticeModel["tabs"][number]["root"]): number => (node.type === "pane" ? 1 : count(node.first) + count(node.second));
  return count(tabOf(model, "t1")!.root);
};

export const CHAPTER_ONE: PracticeChapter = {
  id: "screen",
  number: 1,
  title: "화면 나누고 옮기기",
  version: 1,
  intro: "연습 프로젝트에서 에이전트 하나가 결제 오류를 고치고 있어요. 옆에서 테스트를 돌려 보고 싶다면?",
  outro: "나누기, 옮겨 가기, 크게 보기, 새 탭, 그룹까지. 왼쪽 목록이 프로젝트 › main › 그룹 › 탭으로 따라 바뀌어요.",
  model: chapterOneModel,
  steps: [
    { id: "split", title: "나누기", say: "에이전트 옆에 터미널 하나 더", target: '[data-pane-view="lead"]', side: "bottom", command: "split_right", done: (model) => paneCount(model) === 2 },
    { id: "jump", title: "에이전트로 돌아가기", say: "목록 1번 에이전트로 바로 가요. pane을 클릭해도 돼요", target: '[data-pane-view="lead"]', side: "bottom", command: "select_agent_1", done: (model) => paneCount(model) === 2 && focusedPane(model) === "lead" },
    { id: "zoom", title: "크게 보기", say: "이 pane만 크게 봐요", target: '[data-pane-view="lead"]', side: "bottom", command: "toggle_zoom", done: (model) => tabOf(model, "t1")!.zoomed },
    { id: "unzoom", title: "원래대로", say: "같은 키로 돌아와요. 나눈 모양은 그대로예요", target: '[data-pane-view="lead"]', side: "bottom", command: "toggle_zoom", done: (model) => !tabOf(model, "t1")!.zoomed },
    { id: "tab", title: "새 탭", say: "다른 일은 새 탭에서", target: "[data-new-agent-tab]", side: "bottom", command: "new_tab", done: (model) => model.tabs.length === 2 },
    { id: "group", title: "그룹 만들기 (탭 우클릭)", say: "탭을 우클릭하면 메뉴가 있어요. 이 탭으로 그룹을 만들어 볼게요", target: '[data-tab-menu="t1"]', side: "bottom", gesture: "contextmenu", menuItem: "groups.newWithTab", fill: { selector: "input", value: "결제 작업" }, done: (model) => model.groups.some((group) => group.name === "결제 작업") },
  ],
};

export const CHAPTERS: readonly PracticeChapter[] = [CHAPTER_ONE];
