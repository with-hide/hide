import type { Catalog, Catalogs } from "../../schema";

// The several-agents guide's words (`practice/guides.ts`).
const en = {
  "practice.agents.title": "Run several agents",
  "practice.agents.summary": "Open agents beside each other, split panes, answer what waits, go back and forth",
  "practice.agents.line": "Ask in words or split a pane yourself. What a lead starts for part of its job is its child, listed under it.",
  "practice.agents.frame1": "A lead starts children for parts of its job; they list under it",
  "practice.agents.frame2": "Split a pane to open another agent beside it",
  "practice.agents.frame3": "What waits on you rises to Needs You",
  "practice.agents.frame4": "Go back and forth between agents; a closed tab can be reopened",
};

const ko = {
  "practice.agents.title": "에이전트 여러 개 돌리기",
  "practice.agents.summary": "에이전트를 나란히 띄우고, 화면을 나누고, 기다리는 것에 답하고, 오가요",
  "practice.agents.line": "말로 시키거나 직접 화면을 나눠요. 리더가 일의 일부로 띄운 에이전트는 그 자식이고, 그 아래에 이어져요.",
  "practice.agents.frame1": "리더가 일의 일부를 자식에게 맡기면 그 아래에 이어져요",
  "practice.agents.frame2": "화면을 나눠 옆에 에이전트를 띄워요",
  "practice.agents.frame3": "나를 기다리는 일은 내 차례로 올라와요",
  "practice.agents.frame4": "에이전트 사이를 오가고, 닫은 탭은 다시 열 수 있어요",
} satisfies Catalog<typeof en>;

const zhCN = {
  "practice.agents.title": "同时运行多个智能体",
  "practice.agents.summary": "把智能体并排打开、拆分窗格、回答等待你的事、来回切换",
  "practice.agents.line": "可以用话来开，也可以自己拆分窗格。主智能体为任务的一部分启动的就是它的子智能体，排在它下面。",
  "practice.agents.frame1": "主智能体把任务的一部分交给子智能体，它们排在其下",
  "practice.agents.frame2": "拆分窗格，在旁边再开一个智能体",
  "practice.agents.frame3": "等你的事会升到“轮到你”",
  "practice.agents.frame4": "在智能体之间来回切换；关闭的标签页可以重开",
} satisfies Catalog<typeof en>;

const ja = {
  "practice.agents.title": "複数のエージェントを動かす",
  "practice.agents.summary": "エージェントを並べて開き、画面を分け、待っているものに答え、行き来します",
  "practice.agents.line": "言葉で頼むか、自分で画面を分けます。リードが仕事の一部として起動したものはその子で、その下に並びます。",
  "practice.agents.frame1": "リードが仕事の一部を子に任せると、その下に並びます",
  "practice.agents.frame2": "画面を分けて、横にエージェントを開きます",
  "practice.agents.frame3": "自分を待っている仕事は「自分の番」に上がります",
  "practice.agents.frame4": "エージェント間を行き来し、閉じたタブは開き直せます",
} satisfies Catalog<typeof en>;

export default { en, ko, "zh-CN": zhCN, ja } satisfies Catalogs<typeof en>;
