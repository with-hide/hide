// The files of a reply on the phone (Doop 08, 09): each uploaded one phone message at a time, at most four chunks out that the
// computer has not acknowledged; a cancel, a lost connection, a failed file and the relay's daily limit each end as the operator
// can act on; a sent reply's thumbnails stay for its own conversation.
import { afterEach, beforeEach, expect, it } from "vitest";
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
  readyStages,
  removeFile,
  reserveFile,
  resetUploads,
  retryFile,
  reuploadFiles,
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
  // Four chunks (WINDOW) go before any is acknowledged, each one phone message as JSON.
  expect(ofType("attach_chunk").map((chunk) => chunk.seq)).toEqual([0, 1, 2, 3]);
  for (const chunk of ofType("attach_chunk")) expect(new TextEncoder().encode(JSON.stringify(chunk)).length).toBeLessThanOrEqual(MAX_FRAME_BYTES);
  expect(readyStages()).toBeNull();

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
  expect(readyStages()).toEqual([begin.stage]);
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
  expect(readyStages()).toEqual([first!.stage, second.stage]);
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

it("drops only the file whose × was pressed, tells the computer to drop it, and goes on with the next", () => {
  const first = attach("a.log", CHUNK * 3);
  const second = attach("b.log", 2);
  const stage = ofType("attach_begin")[0]!.stage;
  removeFile(first);
  expect(ofType("attach_cancel")).toEqual([{ type: "attach_cancel", stage }]);
  expect(usePhone.getState().files.map((file) => file.id)).toEqual([second]);
  // An answer for the cancelled stage that was already on its way changes nothing.
  onAttachAck({ stage, seq: 0 });
  expect(ofType("attach_begin")[1]!.name).toBe("b.log");
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
  expect(readyStages()).toBeNull();

  retryFile(failing);
  const again = ofType("attach_begin")[2]!;
  expect(again.stage).not.toBe(refused);
  onAttachAck({ stage: again.stage, done: true });
  expect(readyStages()).toEqual([again.stage, next.stage]);
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
  const again = ofType("attach_begin")[1]!;
  expect(again.stage).not.toBe(first);
  expect(readyStages()).toBeNull();
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

it("draws this phone's thumbnails only in the conversation it sent them to, by stage or by the same words, and names any other file", () => {
  const key = { device_id: "local", pane_id: "w1:p1" };
  attach("screen.jpg", 4, "photo");
  const stage = ofType("attach_begin")[0]!.stage;
  onAttachAck({ stage, done: true });
  filesSent(key, "차이 나는 부분 봐 줘.");
  expect(usePhone.getState().files).toEqual([]);

  const recorded = `"/state/attachments/${stage}-screen.jpg" 차이 나는 부분 봐 줘.`;
  const mine = shownFiles(key, recorded, Date.now());
  expect(mine.words).toBe("차이 나는 부분 봐 줘.");
  expect(mine.files).toEqual([expect.objectContaining({ name: "screen.jpg", kind: "photo", thumb: "data:image/jpeg;base64,AA==" })]);
  // An agent that records an image as its own says nothing of the path; the same words a moment later are the same reply.
  expect(shownFiles(key, "차이 나는 부분 봐 줘.", Date.now()).files).toHaveLength(1);
  expect(shownFiles(key, "차이 나는 부분 봐 줘.", Date.now() + 10 * 60_000).files).toEqual([]);
  // Another agent's conversation, and an old record, show the file by its name alone.
  expect(shownFiles({ device_id: "local", pane_id: "w1:p2" }, recorded, Date.now()).files).toEqual([{ name: "screen.jpg", stage }]);
});
