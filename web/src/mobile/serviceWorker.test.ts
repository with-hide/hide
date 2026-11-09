import { readFileSync } from "node:fs";
import { createContext, runInContext } from "node:vm";
import { expect, it } from "vitest";

// Only browser APIs are substituted. Run the shipped worker itself and
// inspect the notifications a phone receives, not a copy of its formatter.
function phoneWorker() {
  type Event = { data: unknown; waitUntil: (work: Promise<unknown>) => void };
  const listeners = new Map<string, (event: Event) => void>();
  const stored = new Map<string, Response>();
  const shown: { title: string; body: string; tag: string }[] = [];
  const closed: string[] = [];
  const self = {
    addEventListener: (kind: string, listener: (event: Event) => void) => listeners.set(kind, listener),
    registration: {
      showNotification: async (title: string, options: { body: string; tag: string }) => { shown.push({ title, body: options.body, tag: options.tag }); },
      getNotifications: async () => [{ tag: "old", close: () => closed.push("old") }],
    },
  };
  const caches = { open: async () => ({
    put: async (key: string, value: Response) => { stored.set(key, value); },
    match: async (key: string) => stored.get(key)?.clone(),
  }) };
  runInContext(readFileSync(new URL("../../public/m/sw.js", import.meta.url), "utf8"), createContext({ self, caches, Response, URL }));
  async function dispatch(kind: string, data: unknown) {
    const pending: Promise<unknown>[] = [];
    listeners.get(kind)!({ data, waitUntil: (work) => pending.push(work) });
    await Promise.all(pending);
  }
  return { shown, closed,
    words: (words: Record<string, string>) => dispatch("message", { type: "words", words }),
    push: (data: Record<string, unknown>) => dispatch("push", { json: () => data }),
  };
}

it("shows the row's status and exact sentence, replaces by the same tag and withdraws seen notices", async () => {
  const phone = phoneWorker();
  await phone.words({ needs_you: "Needs your attention", done: "Done", status_question: "Question", status_approval: "Approval", status_error: "Blocked", status_done: "Done", status_unfinished: "Unfinished" });
  for (const [status_code, detail] of [["question", "배포할 환경을 골라 주세요"], ["approval", "계획 승인 대기"], ["error", "로그인이 필요해요"], ["done", "배포 완료"], ["unfinished", "테스트 세 개 남음"]]) {
    await phone.push({ title: "Add a dark mode switch", state: status_code === "done" || status_code === "unfinished" ? "done" : "needs_you", status_code, detail, place: "my-app", tag: "node|w1:p1", clear: ["old"] });
  }
  expect(phone.shown).toEqual([
    { title: "Add a dark mode switch", body: "Question · 배포할 환경을 골라 주세요 · my-app", tag: "node|w1:p1" },
    { title: "Add a dark mode switch", body: "Approval · 계획 승인 대기 · my-app", tag: "node|w1:p1" },
    { title: "Add a dark mode switch", body: "Blocked · 로그인이 필요해요 · my-app", tag: "node|w1:p1" },
    { title: "Add a dark mode switch", body: "Done · 배포 완료 · my-app", tag: "node|w1:p1" },
    { title: "Add a dark mode switch", body: "Unfinished · 테스트 세 개 남음 · my-app", tag: "node|w1:p1" },
  ]);
  expect(phone.closed).toEqual(["old", "old", "old", "old", "old"]);
});

it("words a View to look at as the eye with the View's name, and an older page's words as the section (status-model.md, A View to look at)", async () => {
  const phone = phoneWorker();
  await phone.words({ needs_you: "내 차례", done: "읽을 것", status_look: "볼 것" });
  await phone.push({ title: "task", state: "needs_you", status_code: "look", detail: "report.html", place: "project" });
  await phone.words({ needs_you: "내 차례", done: "읽을 것" });
  await phone.push({ title: "task", state: "needs_you", status_code: "look", detail: "report.html", place: "project" });
  expect(phone.shown.map(({ body }) => body)).toEqual(["볼 것 · report.html · project", "내 차례 · report.html · project"]);
});

it("words a question the core asks for the agent from its subject, in the words the page posted (hide-plan B14)", async () => {
  const phone = phoneWorker();
  await phone.words({ needs_you: "Needs your attention", done: "Done", status_question: "Question", subject_plan_review: "Plan review · {{count}} steps" });
  await phone.push({ title: "task", state: "needs_you", status_code: "question", detail: null, subject: { cause: "plan_review", steps: 5 }, place: "project" });
  await phone.words({ needs_you: "내 차례", done: "읽을 것", status_question: "질문" });
  await phone.push({ title: "task", state: "needs_you", status_code: "question", detail: null, subject: { cause: "plan_review", steps: 5 }, place: "project" });
  expect(phone.shown.map(({ body }) => body)).toEqual(["Question · Plan review · 5 steps · project", "질문 · project"]);
});

it("words a mission decision as 판단 and its line, and an unknown cause as the payload's sentence (needs-you-cleanup B4)", async () => {
  const phone = phoneWorker();
  const decision = { cause: "mission_decision", line: "Skip the check line?" };
  await phone.words({ needs_you: "내 차례", done: "읽을 것", status_question: "질문", subject_mission_decision: "판단" });
  await phone.push({ title: "task", state: "needs_you", status_code: "question", detail: "Skip the check…", subject: decision, place: "project" });
  // Words cached by a page from before the cause keep the row's own word.
  await phone.words({ needs_you: "내 차례", done: "읽을 것", status_question: "질문" });
  await phone.push({ title: "task", state: "needs_you", status_code: "question", detail: "Skip the check…", subject: decision, place: "project" });
  // A cause this worker does not know draws the row's plain sentence.
  await phone.push({ title: "task", state: "needs_you", status_code: "question", detail: "Plain sentence", subject: { cause: "later_cause" }, place: "project" });
  expect(phone.shown.map(({ body }) => body)).toEqual(["판단 · Skip the check line? · project", "질문 · Skip the check line? · project", "질문 · Plain sentence · project"]);
});

it("words a mission node raised to the operator as 확인 필요, its title and why (PRD mission-node-simple B14)", async () => {
  const phone = phoneWorker();
  const raised = { cause: "mission_attention", node: "device-row-simple", reason: "worker_ended" };
  await phone.words({ needs_you: "내 차례", done: "읽을 것", status_question: "질문", subject_mission_attention: "확인 필요", subject_mission_attention_worker_ended: "일꾼 끝남, 일 남음" });
  await phone.push({ title: "task", state: "needs_you", status_code: "question", detail: "device-row-simple", subject: raised, place: "project" });
  // Words cached by a page from before the cause keep the row's own word and sentence.
  await phone.words({ needs_you: "내 차례", done: "읽을 것", status_question: "질문" });
  await phone.push({ title: "task", state: "needs_you", status_code: "question", detail: "device-row-simple", subject: raised, place: "project" });
  expect(phone.shown.map(({ body }) => body)).toEqual(["확인 필요 · device-row-simple · 일꾼 끝남, 일 남음 · project", "질문 · device-row-simple · project"]);
});

it("words an unconfirmed brief by which input it was and whether its composer held it", async () => {
  const phone = phoneWorker();
  await phone.words({ needs_you: "내 차례", done: "읽을 것", status_idle: "대기", subject_input_unconfirmed_brief: "브리프 확인 안 됨", subject_input_unconfirmed_letter: "편지 확인 안 됨", subject_input_unconfirmed_brief_held: "브리프 못 받음", subject_input_unconfirmed_letter_held: "편지 못 받음" });
  await phone.push({ title: "task", state: "needs_you", status_code: "idle", detail: "Waiting", subject: { cause: "input_unconfirmed", input: "brief", spent: false, held: false }, place: "project" });
  await phone.push({ title: "task", state: "needs_you", status_code: "idle", detail: "Waiting", subject: { cause: "input_unconfirmed", input: "letter", spent: true, held: true }, place: "project" });
  expect(phone.shown.map(({ body }) => body)).toEqual(["대기 · 브리프 확인 안 됨 · project", "대기 · 편지 못 받음 · project"]);
});

it("words a person's turn as 사람 차례, its kind, where and the device it waits on (PRD ask-human B13)", async () => {
  const phone = phoneWorker();
  await phone.words({ needs_you: "내 차례", done: "읽을 것", status_question: "질문", subject_human_turn: "사람 차례", subject_human_turn_auth: "인증", subject_human_turn_permission: "허용" });
  await phone.push({ title: "task", state: "needs_you", status_code: "question", detail: "Sign in", subject: { cause: "human_turn", id: "a.1", kind: "auth", place: "github.com/login", device: "mini" }, place: "project" });
  await phone.push({ title: "task", state: "needs_you", status_code: "question", detail: "Allow", subject: { cause: "human_turn", id: "a.2", kind: "permission", place: "System Settings > Camera" }, place: "project" });
  expect(phone.shown.map(({ body }) => body)).toEqual(["사람 차례 · 인증 · github.com/login · mini · project", "사람 차례 · 허용 · System Settings > Camera · project"]);
});

it("leads with the status word with no detail too, and works with older payloads and cached page words", async () => {
  const phone = phoneWorker();
  await phone.words({ needs_you: "내 차례", done: "읽을 것", status_done: "끝남", status_question: "질문", status_stopped: "멈춤" });
  await phone.push({ title: "task", state: "needs_you", place: "project" });
  await phone.push({ title: "task", state: "done", status_code: "stopped", detail: null, place: "project" });
  await phone.push({ title: "task", state: "needs_you", status_code: "question", detail: "Choose staging?", place: "" });
  await phone.push({ title: "task", state: "done", status_code: "done", detail: "배포 완료", place: "project" });
  await phone.words({ needs_you: "내 차례", done: "읽을 것" });
  await phone.push({ title: "task", state: "needs_you", status_code: "question", detail: "Choose staging?", place: "project" });
  expect(phone.shown.map(({ body }) => body)).toEqual(["내 차례 · project", "멈춤 · project", "질문 · Choose staging?", "끝남 · 배포 완료 · project", "내 차례 · Choose staging? · project"]);
  const newPhone = phoneWorker();
  await newPhone.push({ title: "task", state: "done", detail: "배포 완료", place: "project" });
  expect(newPhone.shown).toEqual([{ title: "task", body: "배포 완료 · project", tag: undefined }]);
});

it("a hosted push stays bound to its registration's device and opens that agent in the loader", async () => {
  for (const room of ["a".repeat(43), "b".repeat(43)]) {
    const handlers = new Map<string, (event: unknown) => void>();
    let notification: { data: unknown } | null = null;
    const opened: string[] = [];
    const self = {
      location: new URL("https://app.withhide.dev/sw.js"),
      registration: { scope: `https://app.withhide.dev/push/${room}/`, getNotifications: async () => [],
        showNotification: async (_title: string, options: { data: unknown }) => { notification = options; } },
      clients: { matchAll: async () => [], openWindow: async (url: string) => { opened.push(url); } },
      addEventListener: (kind: string, listener: (event: unknown) => void) => handlers.set(kind, listener),
    };
    const caches = { open: async () => ({ match: async () => undefined }) };
    runInContext(readFileSync(new URL("../../public/m/sw.js", import.meta.url), "utf8"), createContext({ self, caches, Response, URL, URLSearchParams }));
    const work: Promise<unknown>[] = [];
    const waitUntil = (promise: Promise<unknown>) => work.push(promise);
    handlers.get("push")!({ data: { json: () => ({ title: "Ready", device_id: "device", pane_id: "pane", room: "untrusted" }) }, waitUntil });
    await Promise.all(work);
    handlers.get("notificationclick")!({ notification: { ...notification!, close: () => {} }, waitUntil });
    await Promise.all(work);
    expect(opened).toEqual([`/?host=${room}&open=device%7Cpane`]);
  }
});
