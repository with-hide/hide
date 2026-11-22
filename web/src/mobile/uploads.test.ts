// The files of a reply on the phone (Doop 08, 09): each uploaded one phone message at a time, at most four upload frames out that the
// computer has not answered, across files and cancels; a cancel, a lost connection, a failed file and the relay's daily limit each
// end as the operator can act on; a reply takes only the files it named, and its thumbnails stay for its own conversation.
import { afterEach, beforeEach, expect, it } from "vitest";
import { attachPicked } from "./pickFiles";
import { MAX_FRAME_BYTES, type PhoneMessage } from "./protocol";
import { patch, usePhone } from "./store";
import {
  BATCH_BYTES,
  CHUNK,
  FILE_BYTES,
  addFile,
  attachedOf,
  bindUploads,
  fileName,
  filesSent,
  onAttachAck,
  removeFile,
  replyFiles,
  reserveFile,
  resetUploads,
  retryFile,
  reuploadFiles,
  SENT_REPLIES,
  shownFiles,
  uploadsConnected,
  uploadsLost,
} from "./uploads";

let sent: PhoneMessage[] = [];

beforeEach(() => {
  sent = [];
  resetUploads();
  patch({ connected: true, files: [], fileNotice: null });
  bindUploads((message) => {
    if (!usePhone.getState().connected) return false;
    sent.push(message);
    return true;
  });
});

afterEach(() => resetUploads());

function attach(name: string, size: number, kind: "photo" | "file" = "file"): string {
  const id = reserveFile(name, kind)!;
  addFile(id, { name, bytes: new Uint8Array(size).fill(7), kind, thumb: kind === "photo" ? "data:image/jpeg;base64,AA==" : null });
  return id;
}

const ofType = <T extends PhoneMessage["type"]>(type: T) => sent.filter((message): message is Extract<PhoneMessage, { type: T }> => message.type === type);
const state = (id: string) => usePhone.getState().files.find((file) => file.id === id);

it("uploads a file in chunks of one phone message, four out at most, and names its stage once the computer has every byte", () => {
  const size = CHUNK * 6 + 5;
  const id = attach("holder.log", size);
  const begin = ofType("attach_begin")[0]!;
  expect(begin).toEqual({ type: "attach_begin", stage: begin.stage, name: "holder.log", size });
  expect(begin.stage).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
  // Four upload frames (WINDOW) go before any is answered, the begin among them, each one phone message as JSON.
  expect(ofType("attach_chunk").map((chunk) => chunk.seq)).toEqual([0, 1, 2]);
  for (const chunk of ofType("attach_chunk")) expect(new TextEncoder().encode(JSON.stringify(chunk)).length).toBeLessThanOrEqual(MAX_FRAME_BYTES);
  expect(replyFiles()).toBeNull();

  onAttachAck({ stage: begin.stage, begun: true });
  expect(ofType("attach_chunk").map((chunk) => chunk.seq)).toEqual([0, 1, 2, 3]);
  onAttachAck({ stage: begin.stage, seq: 0 });
  expect(ofType("attach_chunk").map((chunk) => chunk.seq)).toEqual([0, 1, 2, 3, 4]);
  expect(state(id)!.sent).toBe(CHUNK);
  for (let seq = 1; seq <= 6; seq += 1) onAttachAck({ stage: begin.stage, seq });
  // The last chunk is shorter, and the end follows it.
  expect(ofType("attach_chunk")).toHaveLength(7);
  expect(atob(ofType("attach_chunk")[6]!.data)).toHaveLength(5);
  expect(ofType("attach_end")).toEqual([{ type: "attach_end", stage: begin.stage }]);
  expect(state(id)!.state).toBe("uploading");

  onAttachAck({ stage: begin.stage, done: true });
  expect(state(id)).toMatchObject({ state: "ready", sent: size });
  expect(replyFiles()?.stages).toEqual([begin.stage]);
});

it("uploads the files one after another in the order picked, and a reply names them in that order", () => {
  attach("a.log", 3);
  attach("b.log", 3);
  const [first] = ofType("attach_begin");
  expect(ofType("attach_begin")).toHaveLength(1);
  onAttachAck({ stage: first!.stage, seq: 0 });
  onAttachAck({ stage: first!.stage, done: true });
  const second = ofType("attach_begin")[1]!;
  expect(second.name).toBe("b.log");
  onAttachAck({ stage: second.stage, seq: 0 });
  onAttachAck({ stage: second.stage, done: true });
  expect(replyFiles()?.stages).toEqual([first!.stage, second.stage]);
});

it("leaves out a file past 10 MB, or past the reply's 40 MB, with one line, and keeps the others", () => {
  const kept = attach("small.log", 10);
  attach("trace.json", FILE_BYTES + 1);
  expect(usePhone.getState().fileNotice).toEqual({ kind: "too_large", name: "trace.json", size: FILE_BYTES + 1 });
  expect(usePhone.getState().files.map((file) => file.id)).toEqual([kept]);
  for (let index = 0; index < 3; index += 1) attach(`big-${index}.bin`, FILE_BYTES);
  attach("over.bin", BATCH_BYTES - 3 * FILE_BYTES);
  expect(usePhone.getState().fileNotice).toEqual({ kind: "too_much" });
  expect(usePhone.getState().files).toHaveLength(4);
  // An eighth file is the last one a reply attaches.
  for (let index = 0; index < 4; index += 1) attach(`n-${index}.txt`, 1);
  expect(reserveFile("ninth.txt", "file")).toBeNull();
  expect(usePhone.getState().fileNotice).toEqual({ kind: "too_many" });
});

/** Answers the oldest upload frame not yet answered, as the computer does, in order; returns false when none is out. */
function answerNext(answered: { count: number }): boolean {
  const uploads = sent.filter((message) => message.type.startsWith("attach_"));
  const frame = uploads[answered.count];
  if (!frame || !("stage" in frame)) return false;
  answered.count += 1;
  if (frame.type === "attach_begin") onAttachAck({ stage: frame.stage, begun: true });
  else if (frame.type === "attach_chunk") onAttachAck({ stage: frame.stage, seq: frame.seq });
  else if (frame.type === "attach_end") onAttachAck({ stage: frame.stage, done: true });
  else onAttachAck({ stage: frame.stage, cancelled: true });
  return true;
}

const outstanding = (answered: { count: number }) => sent.filter((message) => message.type.startsWith("attach_")).length - answered.count;

it("drops only the file whose × was pressed, and keeps four frames out at most across its cancel and the next file", () => {
  const first = attach("a.log", CHUNK * 8);
  const second = attach("b.log", CHUNK * 8);
  const stage = ofType("attach_begin")[0]!.stage;
  const answered = { count: 0 };
  expect(outstanding(answered)).toBe(4);
  // Pressed before any answer: the cancel and the next file wait for the frames already out to be answered.
  removeFile(first);
  expect(usePhone.getState().files.map((file) => file.id)).toEqual([second]);
  expect(ofType("attach_cancel")).toEqual([]);
  expect(ofType("attach_begin")).toHaveLength(1);
  while (answerNext(answered)) expect(outstanding(answered)).toBeLessThanOrEqual(4);
  // The cancel went before the next file began, and that file went whole.
  const order = sent.filter((message) => message.type === "attach_cancel" || message.type === "attach_begin").map((message) => message.type);
  expect(order).toEqual(["attach_begin", "attach_cancel", "attach_begin"]);
  expect(ofType("attach_cancel")).toEqual([{ type: "attach_cancel", stage }]);
  expect(state(second)).toMatchObject({ state: "ready", sent: CHUNK * 8 });
});

it("fails only the file the computer refused; pressing it uploads it again under a new stage, and the reply waits for it", () => {
  const failing = attach("a.log", 2);
  const fine = attach("b.log", 2);
  const refused = ofType("attach_begin")[0]!.stage;
  onAttachAck({ stage: refused, failed: "staging_full" });
  expect(state(failing)).toMatchObject({ state: "failed", reason: "staging_full" });
  expect(usePhone.getState().fileNotice).toEqual({ kind: "failed" });
  const next = ofType("attach_begin")[1]!;
  onAttachAck({ stage: next.stage, done: true });
  expect(state(fine)!.state).toBe("ready");
  expect(replyFiles()).toBeNull();

  retryFile(failing);
  const again = ofType("attach_begin")[2]!;
  expect(again.stage).not.toBe(refused);
  onAttachAck({ stage: again.stage, done: true });
  expect(replyFiles()?.stages).toEqual([again.stage, next.stage]);
});

it("uploads every unsent file again under new stages once a lost connection is back, since the computer released them", () => {
  const done = attach("a.log", 2);
  const first = ofType("attach_begin")[0]!.stage;
  onAttachAck({ stage: first, done: true });
  attach("b.log", CHUNK * 2);
  patch({ connected: false });
  uploadsLost(null);
  expect(usePhone.getState().files.map((file) => [file.state, file.sent])).toEqual([["uploading", 0], ["uploading", 0]]);
  sent = [];
  patch({ connected: true });
  uploadsConnected();
  const restarted = ofType("attach_begin")[0]!;
  expect(restarted.name).toBe("a.log");
  expect(restarted.stage).not.toBe(first);
  onAttachAck({ stage: restarted.stage, done: true });
  expect(state(done)!.state).toBe("ready");
});

it("ends the uploads with the daily limit's line when the relay's day is spent, and starts none on the retries", () => {
  attach("a.log", CHUNK * 2);
  uploadsLost("daily_limit");
  uploadsLost(null);
  expect(usePhone.getState().files.map((file) => file.state)).toEqual(["failed"]);
  expect(usePhone.getState().fileNotice).toEqual({ kind: "daily_limit" });
  sent = [];
  uploadsConnected();
  expect(sent).toEqual([]);
});

it("uploads every file again when the computer refused a reply because it no longer holds one of them", () => {
  attach("a.log", 2);
  const first = ofType("attach_begin")[0]!.stage;
  onAttachAck({ stage: first, done: true });
  reuploadFiles();
  // The stage the computer may still hold is dropped before the file goes up again.
  expect(ofType("attach_cancel")).toEqual([{ type: "attach_cancel", stage: first }]);
  const again = ofType("attach_begin")[1]!;
  expect(again.stage).not.toBe(first);
  expect(replyFiles()).toBeNull();
});

it("keeps a name the computer takes as one file name, its extension kept", () => {
  expect(fileName("../../etc/passwd")).toBe(".._.._etc_passwd");
  expect(fileName("a\u0000b.log")).toBe("a_b.log");
  const long = `${"메모리 사용량 측정 ".repeat(10)}.log`;
  const kept = fileName(long);
  expect(new TextEncoder().encode(kept).length).toBeLessThanOrEqual(96);
  expect(kept.endsWith(".log")).toBe(true);
});

it("reads the quoted paths in front of an operator's message as its files and the rest as its words", () => {
  const stage = "01234567-0123-4123-8123-0123456789ab";
  const text = `"/Users/me/.hide/state/attachments/${stage}-holder.log" "C:\\\\Users\\\\me\\\\shot \\"1\\".png" 차이 나는 부분 봐 줘.`;
  expect(attachedOf(text)).toEqual({
    files: [
      { name: "holder.log", stage },
      { name: 'shot "1".png', stage: null },
    ],
    words: "차이 나는 부분 봐 줘.",
  });
  expect(attachedOf('"not a path" 그대로')).toEqual({ files: [], words: '"not a path" 그대로' });
});

/** Uploads one file to the end and sends it with a reply; its stage. */
function sendOne(key: { device_id: string; pane_id: string }, name: string, kind: "photo" | "file" = "file"): string {
  const id = attach(name, 4, kind);
  const stage = ofType("attach_begin").at(-1)!.stage;
  onAttachAck({ stage, begun: true });
  onAttachAck({ stage, seq: 0 });
  onAttachAck({ stage, done: true });
  filesSent(key, [id]);
  return stage;
}

it("draws this phone's thumbnails only for the reply whose stage the message names, in its own conversation, and names any other file", () => {
  const key = { device_id: "local", pane_id: "w1:p1" };
  const stage = sendOne(key, "screen.jpg", "photo");
  expect(usePhone.getState().files).toEqual([]);

  const recorded = `"/state/attachments/${stage}-screen.jpg" 확인해 줘.`;
  const mine = shownFiles(key, recorded);
  expect(mine.words).toBe("확인해 줘.");
  expect(mine.files).toEqual([expect.objectContaining({ name: "screen.jpg", kind: "photo", thumb: "data:image/jpeg;base64,AA==" })]);
  // The same words sent again with no file are another reply, which attached nothing.
  expect(shownFiles(key, "확인해 줘.")).toEqual({ files: [], words: "확인해 줘." });
  // A second photo under the same words is its own reply, found by its own stage.
  const later = sendOne(key, "other.jpg", "photo");
  expect(shownFiles(key, `"/state/attachments/${later}-other.jpg" 확인해 줘.`).files).toEqual([expect.objectContaining({ name: "other.jpg", stage: later })]);
  // Another agent's conversation shows the file by its name alone.
  expect(shownFiles({ device_id: "local", pane_id: "w1:p2" }, recorded).files).toEqual([{ name: "screen.jpg", stage }]);
});

it("takes off the bar only the files the landed reply named, and keeps a file picked while it was on its way", () => {
  const key = { device_id: "local", pane_id: "w1:p1" };
  attach("sent.log", 2);
  const stage = ofType("attach_begin")[0]!.stage;
  onAttachAck({ stage, done: true });
  const reply = replyFiles()!;
  const picked = attach("later.log", CHUNK * 2);
  filesSent(key, reply.ids);
  expect(usePhone.getState().files.map((file) => [file.id, file.state])).toEqual([[picked, "uploading"]]);
  expect(shownFiles(key, `"/state/attachments/${stage}-sent.log" 봐 줘.`).files.map((file) => file.name)).toEqual(["sent.log"]);
});

it("leaves out a photo the phone cannot make smaller with its line, and never sends the original", async () => {
  // Node has no image decoder, as a phone that cannot draw a photo's format has none for it.
  await attachPicked([new File([new Uint8Array([0xff, 0xd8, 0xff, 0xe1])], "IMG_0042.HEIC", { type: "image/heic" })]);
  expect(usePhone.getState().files).toEqual([]);
  expect(usePhone.getState().fileNotice).toEqual({ kind: "photo_failed", name: "IMG_0042.HEIC" });
  expect(ofType("attach_begin")).toEqual([]);
});

it("keeps the files of the last 32 replies, and names an older reply's files from its paths alone", () => {
  const key = { device_id: "local", pane_id: "w1:p1" };
  const stages = Array.from({ length: SENT_REPLIES + 1 }, (_, index) => sendOne(key, `shot-${index}.jpg`, "photo"));
  const message = (index: number) => `"/state/attachments/${stages[index]}-shot-${index}.jpg" 봐 줘.`;
  expect(shownFiles(key, message(0)).files).toEqual([{ name: "shot-0.jpg", stage: stages[0] }]);
  expect(shownFiles(key, message(1)).files).toEqual([expect.objectContaining({ name: "shot-1.jpg", thumb: "data:image/jpeg;base64,AA==" })]);
  expect(shownFiles(key, message(SENT_REPLIES)).files).toEqual([expect.objectContaining({ name: `shot-${SENT_REPLIES}.jpg`, thumb: "data:image/jpeg;base64,AA==" })]);
});
