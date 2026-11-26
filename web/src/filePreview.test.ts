import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { requestFileBytesWithTotal } from "./fileBytes";
import { CAP, clearPreviews, KEPT, MAKING, MAKE_WAIT_MS, TEXT_HEAD, textHead, watchPreview, type Preview } from "./filePreview";
import { drawPreview } from "./previewDraw";

vi.mock("./fileBytes", () => ({ requestFileBytesWithTotal: vi.fn() }));
vi.mock("./previewDraw", () => ({ drawPreview: vi.fn(async () => ({ state: "picture", picture: "data:drawn", pages: 3, seconds: null })) }));

type Read = { path: string; length: number | undefined; answer: (bytes: number[], total: number) => void; fail: () => void };
let reads: Read[] = [];

beforeEach(() => {
  reads = [];
  vi.mocked(requestFileBytesWithTotal).mockImplementation((path, range) => new Promise((resolve, reject) => {
    reads.push({ path, length: range?.length, answer: (bytes, total) => resolve({ bytes: new Uint8Array(bytes), total }), fail: () => reject(new Error("not_found")) });
  }));
});
afterEach(() => {
  clearPreviews();
  vi.useRealTimers();
  vi.mocked(drawPreview).mockClear();
});

/** Watches `path` under its own key and collects what it is told. */
function watch(path: string, key = path) {
  const seen: Preview[] = [];
  const handle = watchPreview(key, path, null, (preview) => seen.push(preview));
  return { seen, handle, last: () => seen.at(-1) ?? handle.now };
}

const settled = () => new Promise((resolve) => setTimeout(resolve, 0));

it("reads a text file's head only and shows its heading and the three lines after it", async () => {
  const notes = watch("/repo/release-notes.md");
  expect(notes.handle.now).toEqual({ state: "reading" });
  expect(reads.map((read) => [read.path, read.length])).toEqual([["/repo/release-notes.md", TEXT_HEAD]]);
  reads[0]!.answer([...new TextEncoder().encode("---\ntitle: x\n---\n# 1.3.0 릴리스 노트\n\n- 기기 연결이 끊긴 뒤\n- Windows 설치\n> 다시 붙을 때\n네 번째 줄")], 9000);
  await settled();
  expect(notes.last()).toEqual({ state: "text", title: "1.3.0 릴리스 노트", lines: ["기기 연결이 끊긴 뒤", "Windows 설치", "다시 붙을 때"] });
});

it("says a file past its kind's cap is too large with its size, reading only its first byte", async () => {
  const film = watch("/repo/full-run.mov");
  reads[0]!.answer([0], CAP.video + 1);
  await settled();
  expect(film.last()).toEqual({ state: "too_large", bytes: CAP.video + 1 });
  expect(reads.map((read) => read.length)).toEqual([1]);
  expect(drawPreview).not.toHaveBeenCalled();
});

it("draws a PDF under its cap from its whole bytes once, however many tiles show it, and says a file it could not read failed", async () => {
  const first = watch("/repo/report.pdf");
  const second = watch("/repo/report.pdf");
  reads[0]!.answer([0], 10);
  await settled();
  reads[1]!.answer([1, 2], 2);
  await settled();
  expect(reads.map((read) => read.length)).toEqual([1, undefined]);
  expect(first.last()).toEqual({ state: "picture", picture: "data:drawn", pages: 3, seconds: null });
  expect(second.last()).toEqual(first.last());
  expect(drawPreview).toHaveBeenCalledTimes(1);

  const gone = watch("/repo/gone.png");
  reads[2]!.fail();
  await settled();
  expect(gone.last()).toEqual({ state: "failed" });
});

it("makes at most two at once, frees a turn nothing answers, and keeps at most 64 previews no tile shows", async () => {
  vi.useFakeTimers();
  for (let index = 0; index < 3; index += 1) watch(`/repo/${index}.md`);
  expect(reads).toHaveLength(MAKING);
  await vi.advanceTimersByTimeAsync(MAKE_WAIT_MS);
  // The two quiet reads gave up and the third file's turn came.
  expect(reads.map((read) => read.path)).toEqual(["/repo/0.md", "/repo/1.md", "/repo/2.md"]);

  const shown = Array.from({ length: KEPT + 1 }, (_, index) => watch(`/repo/kept-${index}.txt`));
  for (const one of shown) one.handle.stop();
  // The first one shown is let go: shown again, it is made again.
  const before = reads.length;
  watch("/repo/kept-0.txt");
  watch(`/repo/kept-${KEPT}.txt`);
  await vi.advanceTimersByTimeAsync(MAKE_WAIT_MS * KEPT);
  expect(reads.slice(before).filter((read) => read.path === "/repo/kept-0.txt")).toHaveLength(1);
  expect(reads.filter((read) => read.path === `/repo/kept-${KEPT}.txt`)).toHaveLength(1);
});

it("stops a make past its time: a read that answers later is followed by no other read and no drawing, and the tile says failed", async () => {
  vi.useFakeTimers();
  const pdf = watch("/repo/late.pdf");
  expect(reads).toHaveLength(1);
  await vi.advanceTimersByTimeAsync(MAKE_WAIT_MS);
  expect(pdf.last()).toEqual({ state: "failed" });
  reads[0]!.answer([37], 1024);
  await vi.advanceTimersByTimeAsync(0);
  expect(reads.filter((read) => read.path === "/repo/late.pdf")).toHaveLength(1);
  expect(drawPreview).not.toHaveBeenCalled();
});

it("reads a heading-less text's first line as its title", () => {
  expect(textHead("first\nsecond\n\nthird\nfourth\nfifth")).toEqual({ title: "first", lines: ["second", "third", "fourth"] });
});
