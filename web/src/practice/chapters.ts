// The practice chapters as data: what each step points at, what it says, which
// command performs it, and the world state that means it is done. A step
// names a command, never a chord, so the label and the key follow the host,
// the system and the operator's own bindings the way every other screen does.

import type { CommandId } from "../shortcuts";
import { focusedPane, PRACTICE_WORLD, type PracticeModel } from "./practiceWorld";
import { TOUR_WORLD, type TourModel } from "./tourWorld";
import type { PracticeWorld } from "./world";

/** How a step is performed when it has no command: a click or a right-click on its target. */
export type StepGesture = "click" | "contextmenu";

export type PracticeStep<M = unknown> = {
  id: string;
  /** The quest card's line; a step without one is part of the step before it. */
  quest?: string;
  /** The bubble: a small line above, the title, and one sentence under it. */
  kick: string;
  title: string;
  text: string;
  /** What the spotlight rings: a selector into the real shell; every match is ringed as one box. */
  target?: string;
  /** Where the bubble sits beside the target; `center` floats it low in the middle. */
  side: "right" | "left" | "bottom" | "inside" | "center";
  /** The three parts of the screen, drawn as labeled zones. */
  zones?: true;
  command?: CommandId;
  gesture?: StepGesture;
  /** A real menu item the step goes on to press, by its i18n key, once the gesture has opened the menu. */
  menuItem?: string;
  /** A field the step fills for the learner, so the step stays one press. */
  fill?: { selector: string; value: string };
  /** Done when the world says so; a step without it is a look, done when the learner presses on. */
  done?: (model: M) => boolean;
};

export type PracticeChapter<M = unknown> = {
  number: number;
  title: string;
  /** Bumped when the steps change, so a learner's progress on the old steps reads as changed. */
  version: number;
  world: PracticeWorld<M>;
  steps: PracticeStep<M>[];
  /** Whether the quest card lists the steps; a look-only tour has nothing to tick. */
  quest: boolean;
};

/** Types a chapter against its own world, then lets the list hold chapters of every world. */
const chapter = <M,>(spec: PracticeChapter<M>) => spec as unknown as PracticeChapter;

export const CHAPTER_ZERO = chapter<TourModel>({
  number: 0,
  title: "한눈에 보기",
  version: 1,
  world: TOUR_WORLD,
  quest: false,
  steps: [
    { id: "parts", zones: true, side: "center", kick: "Hide는 세 칸", title: "왼쪽에서 고르고, 가운데서 일하고, 오른쪽에서 프로젝트를 봐요", text: "하나씩 짚어 볼게요." },
    { id: "left", target: "[data-sidebar-box] [data-sidebar]", side: "right", kick: "① 보는 곳", title: "내 에이전트와 프로젝트가 여기 다 모여요", text: "내 차례인 일이 맨 위에 올라오고, 아래로 프로젝트 › 브랜치 › 탭 › 에이전트가 이어져요." },
    { id: "lenses", target: "[data-sidebar-lens]", side: "right", kick: "① 보는 곳", title: "같은 일을 세 가지로 봐요", text: "프로젝트별로, 태스크(할 일)별로, PR별로. 눌러서 끝까지 넘겨 보세요.", command: "toggle_sidebar_lens", done: (model) => model.seen.length === 3 },
    { id: "search", target: "[data-sidebar-search]", side: "right", kick: "① 보는 곳", title: "무엇이든 찾기", text: "세션, PR, 파일까지 한 칸에서 찾아요." },
    { id: "center", target: "[data-agent-area]", side: "right", kick: "② 일하는 곳", title: "에이전트가 일하는 터미널", text: "탭과 pane으로 나눠 여러 에이전트를 나란히 둬요. 직접 나누는 건 1편에서." },
    { id: "lane", target: "[data-lane-slot]", side: "left", kick: "③ 프로젝트와 그 작업", title: "지금 프로젝트가 어떻게 돌아가는지", text: "세션, PR, 워크트리, 비용이 한 판에 있고, 아래 할 일에 나를 기다리는 일이 모여요." },
    { id: "todo", target: "[data-project-todo]", side: "left", kick: "③ 프로젝트와 그 작업", title: "할 일만 보면 돼요", text: "나를 기다리는 에이전트, 머지할 PR. 누르면 그 일로 바로 가요." },
    { id: "tools", target: "[data-lane-tool]", side: "left", kick: "③ 프로젝트와 그 작업", title: "도구 바꾸기", text: "문서 · 프로젝트 · 파일(Explorer) · 바뀐 것(Changes)." },
    { id: "devices", target: "[data-device-rail]", side: "right", kick: "맨 왼쪽", title: "기기", text: "다른 Mac이나 서버에서 도는 에이전트도 같은 화면에 모여요." },
    { id: "end", zones: true, side: "center", kick: "다 봤어요", title: "이제 직접 해 볼까요?", text: "1편에서 화면을 나누고, 크게 보고, 탭을 묶어 봐요." },
  ],
});

const tabOf = (model: PracticeModel, id: string) => model.tabs.find((tab) => tab.id === id);
const paneCount = (model: PracticeModel) => {
  const count = (node: PracticeModel["tabs"][number]["root"]): number => (node.type === "pane" ? 1 : count(node.first) + count(node.second));
  return count(tabOf(model, "t1")!.root);
};

export const CHAPTER_ONE = chapter<PracticeModel>({
  number: 1,
  title: "화면 나누고 옮기기",
  version: 2,
  world: PRACTICE_WORLD,
  quest: true,
  steps: [
    { id: "split", quest: "나누기", target: '[data-pane-view="lead"]', side: "inside", kick: "연습 프로젝트에서 에이전트가 결제 오류를 고치는 중", title: "옆에 터미널 하나 더", text: "에이전트는 그대로 두고, 옆에서 테스트를 돌려 볼게요.", command: "split_right", done: (model) => paneCount(model) === 2 },
    { id: "jump", quest: "에이전트로 돌아가기", target: '[data-pane-view="lead"]', side: "inside", kick: "오른쪽에 새 터미널이 생겼어요", title: "에이전트로 바로 돌아가기", text: "목록 1번 에이전트로 가요. pane을 클릭해도 돼요.", command: "select_agent_1", done: (model) => paneCount(model) === 2 && focusedPane(model) === "lead" },
    { id: "zoom", quest: "크게 보기", target: '[data-pane-view="lead"]', side: "inside", kick: "크게 보기", title: "이 pane만 크게", text: "잠깐 크게 보고 싶을 때.", command: "toggle_zoom", done: (model) => tabOf(model, "t1")!.zoomed },
    { id: "unzoom", quest: "원래대로", target: '[data-pane-view="lead"]', side: "inside", kick: "원래대로", title: "같은 키로 돌아와요", text: "나눈 모양은 그대로 남아 있어요.", command: "toggle_zoom", done: (model) => !tabOf(model, "t1")!.zoomed },
    { id: "tab", quest: "새 탭", target: "[data-new-agent-tab]", side: "bottom", kick: "새 탭", title: "다른 일은 새 탭에서", text: "탭마다 따로 나눠 둘 수 있어요.", command: "new_tab", done: (model) => model.tabs.length === 2 },
    { id: "group", quest: "그룹 만들기", target: '[data-tab-menu="t1"]', side: "bottom", kick: "그룹 만들기", title: "탭을 우클릭해 보세요", text: "탭 메뉴에서 이 탭으로 새 그룹을 만들어요.", gesture: "contextmenu", menuItem: "groups.newWithTab", fill: { selector: "input", value: "결제 작업" }, done: (model) => model.groups.some((group) => group.name === "결제 작업") },
    { id: "end", target: '[data-sidebar-group-select]:not([data-sidebar-group-select="ungrouped"])', side: "right", kick: "1편 끝", title: "왼쪽 목록이 따라 바뀌었어요", text: "프로젝트 › main › 결제 작업 › 탭. 그룹째로 접고 펼칠 수 있어요." },
  ],
});

export const CHAPTERS: readonly PracticeChapter[] = [CHAPTER_ZERO, CHAPTER_ONE];
