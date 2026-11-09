import type { Catalog, Catalogs } from "../../schema";

// The devices guide's words (`practice/guides.ts`).
const en = {
  "practice.devices.title": "Work on more than one computer",
  "practice.devices.summary": "The device rail, the mark that says a computer needs you, and starting work there",
  "practice.devices.line": "Each computer is a tile in the rail. Pick one and the list and the screen are that computer's.",
  "practice.devices.frame1": "Pick a computer in the rail",
  "practice.devices.frame2": "The list shows that computer's agents; a mark says it needs you",
  "practice.devices.frame3": "Start a tab there; it opens on that computer",
};

const ko = {
  "practice.devices.title": "여러 컴퓨터로 일하기",
  "practice.devices.summary": "기기 목록, 컴퓨터가 나를 기다린다는 표시, 거기서 일 시작하기",
  "practice.devices.line": "컴퓨터마다 네모 하나가 있어요. 하나를 고르면 목록과 화면이 그 컴퓨터 것이 돼요.",
  "practice.devices.frame1": "기기 목록에서 컴퓨터를 골라요",
  "practice.devices.frame2": "목록에 그 컴퓨터의 에이전트가 보이고, 표시가 있으면 내 차례예요",
  "practice.devices.frame3": "거기서 탭을 열면 그 컴퓨터에서 열려요",
} satisfies Catalog<typeof en>;

const zhCN = {
  "practice.devices.title": "在多台电脑上工作",
  "practice.devices.summary": "设备栏、提示某台电脑在等你的标记，以及在那里开始工作",
  "practice.devices.line": "每台电脑在设备栏里是一个方块。选一个，列表和屏幕就是那台电脑的。",
  "practice.devices.frame1": "在设备栏里选一台电脑",
  "practice.devices.frame2": "列表显示那台电脑的智能体；有标记就是轮到你",
  "practice.devices.frame3": "在那里开标签页，它就在那台电脑上打开",
} satisfies Catalog<typeof en>;

const ja = {
  "practice.devices.title": "複数のコンピューターで作業する",
  "practice.devices.summary": "デバイスレール、コンピューターが自分を待っている印、そこで作業を始めること",
  "practice.devices.line": "コンピューターごとにタイルが 1 つあります。選ぶと、一覧と画面がそのコンピューターのものになります。",
  "practice.devices.frame1": "レールでコンピューターを選びます",
  "practice.devices.frame2": "一覧にそのコンピューターのエージェントが並び、印があれば自分の番です",
  "practice.devices.frame3": "そこでタブを開くと、そのコンピューターで開きます",
} satisfies Catalog<typeof en>;

export default { en, ko, "zh-CN": zhCN, ja } satisfies Catalogs<typeof en>;
