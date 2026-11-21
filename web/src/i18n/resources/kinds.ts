import type { Catalog, Catalogs } from "../schema";

// What kind of work a page shows: the `미션` and `태스크` marks in the head of a
// mission's page and a plan's page, and the `?` window
// that tells the two apart with a moving picture of each.
export const kindsEnglish = {
  "kind.mission": "Mission",
  "kind.task": "Task",
  "kind.help.open": "How a mission differs from a task",
  "kind.help.title": "How is a mission different from a task?",
  "kind.help.lead": "Both split the work into steps. What differs is who does it.",
  "kind.help.mission.head": "Many at once.",
  "kind.help.mission.body": "A Conductor hands the work out, and a different agent on each node opens its PR.",
  "kind.help.task.head": "One, in turn.",
  "kind.help.task.body": "One agent makes its own plan and finishes the steps one by one.",
  "kind.help.mission.picture": "A Conductor sends work to three workers at the same time",
  "kind.help.task.picture": "One agent finishes four steps in order",
  "kind.help.conductor": "Conductor",
  "kind.help.worker": "Worker",
  "kind.help.step1": "Read the cause",
  "kind.help.step2": "Fix the core",
  "kind.help.step3": "Fix the screen",
  "kind.help.step4": "Test + PR",
  "kind.help.replay": "Watch again",
} as const;

const ko: Catalog<typeof kindsEnglish> = {
  "kind.mission": "미션",
  "kind.task": "태스크",
  "kind.help.open": "미션과 태스크의 차이",
  "kind.help.title": "미션과 태스크는 뭐가 달라요?",
  "kind.help.lead": "둘 다 일을 단계로 나눠 보여 주지만, 누가 하느냐가 다릅니다.",
  "kind.help.mission.head": "여럿이 동시에.",
  "kind.help.mission.body": "Conductor가 일을 나눠 맡기고, 노드마다 다른 에이전트가 PR을 냅니다.",
  "kind.help.task.head": "하나가 차례로.",
  "kind.help.task.body": "에이전트 하나가 자기 계획을 세우고 단계를 하나씩 끝냅니다.",
  "kind.help.mission.picture": "Conductor가 일꾼 셋에게 동시에 일을 보냅니다",
  "kind.help.task.picture": "에이전트 하나가 네 단계를 차례로 끝냅니다",
  "kind.help.conductor": "Conductor",
  "kind.help.worker": "일꾼",
  "kind.help.step1": "원인 읽기",
  "kind.help.step2": "core 고치기",
  "kind.help.step3": "화면 고치기",
  "kind.help.step4": "테스트 + PR",
  "kind.help.replay": "다시 보기",
};

const zh: Catalog<typeof kindsEnglish> = {
  "kind.mission": "协作任务",
  "kind.task": "任务",
  "kind.help.open": "协作任务与任务的区别",
  "kind.help.title": "协作任务和任务有什么不同？",
  "kind.help.lead": "两者都把工作拆成步骤，区别在于由谁来做。",
  "kind.help.mission.head": "多个同时做。",
  "kind.help.mission.body": "Conductor 分派工作，每个节点由不同的智能体提交 PR。",
  "kind.help.task.head": "一个依次做。",
  "kind.help.task.body": "一个智能体自己制定计划，逐步完成每个步骤。",
  "kind.help.mission.picture": "Conductor 同时向三个执行者派发工作",
  "kind.help.task.picture": "一个智能体依次完成四个步骤",
  "kind.help.conductor": "Conductor",
  "kind.help.worker": "执行者",
  "kind.help.step1": "读取原因",
  "kind.help.step2": "修改 core",
  "kind.help.step3": "修改界面",
  "kind.help.step4": "测试 + PR",
  "kind.help.replay": "再看一遍",
};

const ja: Catalog<typeof kindsEnglish> = {
  "kind.mission": "ミッション",
  "kind.task": "タスク",
  "kind.help.open": "ミッションとタスクの違い",
  "kind.help.title": "ミッションとタスクは何が違いますか？",
  "kind.help.lead": "どちらも作業をステップに分けて見せますが、誰が進めるかが違います。",
  "kind.help.mission.head": "複数が同時に。",
  "kind.help.mission.body": "Conductor が作業を割り振り、ノードごとに別のエージェントが PR を出します。",
  "kind.help.task.head": "1つが順番に。",
  "kind.help.task.body": "1つのエージェントが自分で計画を立て、ステップを1つずつ終えます。",
  "kind.help.mission.picture": "Conductor が3人のワーカーへ同時に作業を送ります",
  "kind.help.task.picture": "1つのエージェントが4つのステップを順に終えます",
  "kind.help.conductor": "Conductor",
  "kind.help.worker": "ワーカー",
  "kind.help.step1": "原因を読む",
  "kind.help.step2": "core を直す",
  "kind.help.step3": "画面を直す",
  "kind.help.step4": "テスト + PR",
  "kind.help.replay": "もう一度見る",
};

export const kindsCatalogs = { en: kindsEnglish, ko, "zh-CN": zh, ja } satisfies Catalogs<typeof kindsEnglish>;
