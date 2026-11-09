import type { Catalog, Catalogs } from "../../schema";

// The mission guide's words (`practice/guides.ts`).
const en = {
  "practice.mission.title": "Hand over a mission",
  "practice.mission.summary": "Say one sentence; a Conductor plans, starts workers and brings you its questions",
  "practice.mission.line": "The plan waits for your go. Its questions land in Needs You, and what was decided is kept in Memory.",
  "practice.mission.frame1": "Say what you want in one sentence",
  "practice.mission.frame2": "The workers it starts run under the Conductor",
  "practice.mission.frame3": "Its question lands in Needs You; answer it on the map",
};

const ko = {
  "practice.mission.title": "미션 맡기기",
  "practice.mission.summary": "한 문장을 말하면 Conductor가 계획하고, 일꾼을 띄우고, 질문을 가져와요",
  "practice.mission.line": "계획은 내가 승인해야 시작돼요. 질문은 내 차례에 오고, 정한 것은 Memory에 남아요.",
  "practice.mission.frame1": "원하는 걸 한 문장으로 말해요",
  "practice.mission.frame2": "Conductor가 띄운 일꾼들이 그 아래에서 돌아요",
  "practice.mission.frame3": "질문은 내 차례에 오고, 지도에서 답해요",
} satisfies Catalog<typeof en>;

const zhCN = {
  "practice.mission.title": "交出一个任务",
  "practice.mission.summary": "说一句话，Conductor 会做计划、启动执行者，并把问题带给你",
  "practice.mission.line": "计划要等你批准才开始。它的问题会出现在“轮到你”，定下的事保存在 Memory 里。",
  "practice.mission.frame1": "用一句话说出想要的",
  "practice.mission.frame2": "它启动的执行者在 Conductor 之下运行",
  "practice.mission.frame3": "它的问题出现在“轮到你”，在地图上回答",
} satisfies Catalog<typeof en>;

const ja = {
  "practice.mission.title": "ミッションを任せる",
  "practice.mission.summary": "一文を伝えると、Conductor が計画し、ワーカーを起動し、質問を持ってきます",
  "practice.mission.line": "計画はあなたの承認を待ちます。質問は「自分の番」に届き、決めたことは Memory に残ります。",
  "practice.mission.frame1": "望むことを一文で伝えます",
  "practice.mission.frame2": "起動したワーカーは Conductor の下で動きます",
  "practice.mission.frame3": "質問は「自分の番」に届き、マップで答えます",
} satisfies Catalog<typeof en>;

export default { en, ko, "zh-CN": zhCN, ja } satisfies Catalogs<typeof en>;
