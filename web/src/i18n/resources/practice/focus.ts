import type { Catalog, Catalogs } from "../../schema";

// Flow 7's words (`practice/chapters/focus.ts`).
const en = {
  "practice.focus.title": "Focus on one agent",
  "practice.focus.summary": "See an agent's terminal beside the page it makes, and watch the page change as it works",
  "practice.focus.enter.quest": "Turn on focus mode",
  "practice.focus.enter.kick": "An agent is building a page",
  "practice.focus.enter.title": "Turn on focus mode",
  "practice.focus.enter.text": "The terminal moves to the left and the page the agent made stands beside it.",
  "practice.focus.live.kick": "The agent is working",
  "practice.focus.live.title": "The page changes as it works",
  "practice.focus.live.text": "When the agent edits the heading, the page beside it changes too.",
  "practice.focus.leave.quest": "Leave focus mode",
  "practice.focus.leave.kick": "Back to both agents",
  "practice.focus.leave.title": "Leave focus mode",
  "practice.focus.leave.text": "Both agents are side by side again.",
};

const ko = {
  "practice.focus.title": "한 에이전트에 집중하기",
  "practice.focus.summary": "에이전트의 터미널과 그가 만든 페이지를 나란히 놓고, 고치는 대로 바뀌는 걸 봐요",
  "practice.focus.enter.quest": "집중 모드 켜기",
  "practice.focus.enter.kick": "에이전트가 페이지를 만드는 중",
  "practice.focus.enter.title": "집중 모드 켜기",
  "practice.focus.enter.text": "터미널은 왼쪽으로, 에이전트가 만든 페이지는 그 옆에 와요.",
  "practice.focus.live.kick": "에이전트가 작업 중",
  "practice.focus.live.title": "고치는 대로 바뀌어요",
  "practice.focus.live.text": "에이전트가 제목을 고치면 옆 페이지도 바로 바뀌어요.",
  "practice.focus.leave.quest": "집중 모드 나가기",
  "practice.focus.leave.kick": "다시 둘 다 보기",
  "practice.focus.leave.title": "집중 모드 나가기",
  "practice.focus.leave.text": "두 에이전트가 다시 나란히 보여요.",
} satisfies Catalog<typeof en>;

const zhCN = {
  "practice.focus.title": "专注于一个智能体",
  "practice.focus.summary": "把智能体的终端和它做的页面并排放着，看页面随修改而变化",
  "practice.focus.enter.quest": "开启专注模式",
  "practice.focus.enter.kick": "智能体正在做一个页面",
  "practice.focus.enter.title": "开启专注模式",
  "practice.focus.enter.text": "终端移到左边，智能体做的页面显示在它旁边。",
  "practice.focus.live.kick": "智能体正在工作",
  "practice.focus.live.title": "页面随修改而变化",
  "practice.focus.live.text": "智能体改标题时，旁边的页面也会跟着变。",
  "practice.focus.leave.quest": "退出专注模式",
  "practice.focus.leave.kick": "回到两个智能体",
  "practice.focus.leave.title": "退出专注模式",
  "practice.focus.leave.text": "两个智能体又并排显示了。",
} satisfies Catalog<typeof en>;

const ja = {
  "practice.focus.title": "一つのエージェントに集中する",
  "practice.focus.summary": "エージェントのターミナルと作ったページを並べて、直すそばから変わるのを見ます",
  "practice.focus.enter.quest": "集中モードをオンにする",
  "practice.focus.enter.kick": "エージェントがページを作っています",
  "practice.focus.enter.title": "集中モードをオンにする",
  "practice.focus.enter.text": "ターミナルが左に寄り、エージェントが作ったページがその横に並びます。",
  "practice.focus.live.kick": "エージェントが作業中",
  "practice.focus.live.title": "直すそばからページが変わります",
  "practice.focus.live.text": "エージェントが見出しを直すと、横のページも変わります。",
  "practice.focus.leave.quest": "集中モードを終える",
  "practice.focus.leave.kick": "二つのエージェントに戻る",
  "practice.focus.leave.title": "集中モードを終える",
  "practice.focus.leave.text": "二つのエージェントがまた並んで見えます。",
} satisfies Catalog<typeof en>;

export default { en, ko, "zh-CN": zhCN, ja } satisfies Catalogs<typeof en>;
