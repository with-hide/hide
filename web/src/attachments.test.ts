import { beforeAll, beforeEach, describe, expect, it } from "vitest";
import type { TFunction } from "i18next";
import { createInterfaceI18n } from "./i18n/instance";
import {
  configureAttachments,
  filesRefusal,
  frameChunk,
  preflightRefusal,
  receiveAttachmentRefusal,
  refusalText,
  safeName,
  submitAttachments,
  submitExplorerImage,
  type AttachmentInput,
} from "./attachments";
import { useShellStore } from "./store";
import { configureFileBytes, receiveBytes } from "./fileBytes";

function capture() {
  const events: { kind: string; payload: Record<string, unknown> }[] = [];
  const binaries: Uint8Array[] = [];
  configureAttachments({
    dispatch: (event) => events.push({ kind: event.kind, payload: event.payload }),
    sendBinary: (bytes) => binaries.push(bytes),
  });
  return { events, binaries };
}

function decodeFrame(bytes: Uint8Array): { header: Record<string, unknown>; body: Uint8Array } {
  const length = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).getUint32(0, false);
  const header = JSON.parse(new TextDecoder().decode(bytes.subarray(4, 4 + length)));
  return { header, body: bytes.subarray(4 + length) };
}

function answerRead(request: Record<string, unknown>, total: number, body = new Uint8Array([9])): void {
  const header = new TextEncoder().encode(JSON.stringify({ type: "file_bytes", request_id: request.request_id, path: request.path, offset: 0, total, eof: true }));
  const frame = new Uint8Array(4 + header.length + body.length);
  new DataView(frame.buffer).setUint32(0, header.length, false);
  frame.set(header, 4);
  frame.set(body, 4 + header.length);
  receiveBytes(frame.buffer);
}

let en: TFunction<"translation">;
let ko: TFunction<"translation">;
beforeAll(async () => {
  en = (await createInterfaceI18n("en")).t;
  ko = (await createInterfaceI18n("ko")).t;
});

describe("attachment upload", () => {
  beforeEach(() => {
    useShellStore.setState({ attachmentRefusal: null });
  });

  it("frames a chunk with its header and bytes", () => {
    const frame = frameChunk("stage-1", 42, new Uint8Array([1, 2]), true);
    const { header, body } = decodeFrame(frame);
    expect(header).toEqual({ request_id: "stage-1", offset: 42, eof: true });
    expect(Array.from(body)).toEqual([1, 2]);
  });

  it("stages each file, sends its bytes, then commits the batch", () => {
    const { events, binaries } = capture();
    submitAttachments("pane-1", [{ name: "shot.png", bytes: new Uint8Array([9, 9, 9]) }], false, true);
    expect(events.map((event) => event.kind)).toEqual(["attachment_stage", "attachment_commit"]);
    expect(events[0]!.payload).toMatchObject({ name: "shot.png", size: 3, clipboard: false });
    expect(events[1]!.payload).toMatchObject({ pane_id: "pane-1", bracketed_paste: true, clipboard: false });
    expect(Array.isArray(events[1]!.payload.stages)).toBe(true);
    const { header, body } = decodeFrame(binaries[0]!);
    expect(header).toMatchObject({ offset: 0, eof: true });
    expect(Array.from(body)).toEqual([9, 9, 9]);
  });

  it("stages a clipboard image under the batch id the core reads it from", () => {
    const { events } = capture();
    submitAttachments("pane-2", [{ name: "shot.png", bytes: new Uint8Array([1]) }], true, true);
    const stage = events[0]!.payload.request_id as string;
    expect(stage).toBe(events[1]!.payload.request_id);
    expect(events[0]!.payload).toMatchObject({ clipboard: true });
  });

  it("reads Explorer images from their owning checkout within the attachment byte cap", async () => {
    const { events, binaries } = capture();
    const reads: Record<string, unknown>[] = [];
    configureFileBytes((event) => { reads.push(event.payload); });
    const item = { path: "/project/shot.png", directory: false, device: "device-1", root: "/project" };
    const accepted = submitExplorerImage("pane-1", item, true);
    expect(reads[0]).toMatchObject({ path: item.path, device_id: "device-1", root: "/project", length: 20 * 1024 * 1024 + 1 });
    answerRead(reads[0]!, 1);
    await accepted;
    expect(events[0]?.payload).toMatchObject({ name: "shot.png", clipboard: false });
    expect(events[1]?.payload).toMatchObject({ pane_id: "pane-1", bracketed_paste: true });
    expect(decodeFrame(binaries[0]!).body).toEqual(new Uint8Array([9]));
    const count = events.length;
    const oversized = submitExplorerImage("pane-1", item, false);
    answerRead(reads[1]!, 20 * 1024 * 1024 + 1);
    await oversized;
    expect(events).toHaveLength(count);
    expect(useShellStore.getState().attachmentRefusal?.reason).toBe("too_large");
  });

  it("refuses concurrent Explorer reads and discards an image when the connection generation changes", async () => {
    const { events } = capture();
    const reads: Record<string, unknown>[] = [];
    configureFileBytes((event) => { reads.push(event.payload); });
    const item = { path: "/project/shot.png", directory: false, device: "device-1", root: "/project" };
    const pending = submitExplorerImage("pane-1", item, false);
    await submitExplorerImage("pane-2", item, false);
    expect(reads).toHaveLength(1);
    expect(useShellStore.getState().attachmentRefusal).toEqual({ pane_id: "pane-2", reason: "staging_full" });
    useShellStore.setState({ viewGeneration: useShellStore.getState().viewGeneration + 1 });
    answerRead(reads[0]!, 1);
    await pending;
    expect(events).toHaveLength(0);
    expect(useShellStore.getState().attachmentRefusal).toEqual({ pane_id: "pane-1", reason: "no_pane" });
    const retry = submitExplorerImage("pane-1", item, false);
    answerRead(reads[1]!, 1);
    await retry;
    expect(events.at(-1)?.kind).toBe("attachment_commit");
  });

  it("refuses a batch past the daemon's caps before uploading it", () => {
    const bytes = (size: number): AttachmentInput => ({ name: "x", bytes: new Uint8Array(size) });
    expect(preflightRefusal([bytes(1)])).toBeNull();
    expect(preflightRefusal([bytes(20 * 1024 * 1024 + 1)])).toBe("too_large");
    expect(preflightRefusal(Array.from({ length: 9 }, () => bytes(1)))).toBe("too_many_files");
    expect(preflightRefusal([bytes(20 * 1024 * 1024), bytes(20 * 1024 * 1024), bytes(1)])).toBe("batch_too_large");
  });

  it("refuses an over-cap drop from the file sizes before reading it", () => {
    const sized = (size: number) => {
      const value = new File([new Uint8Array(0)], "x", { type: "application/octet-stream" });
      Object.defineProperty(value, "size", { value: size });
      return value;
    };
    expect(filesRefusal([sized(1)])).toBeNull();
    expect(filesRefusal([sized(20 * 1024 * 1024 + 1)])).toBe("too_large");
    expect(filesRefusal(Array.from({ length: 9 }, () => sized(1)))).toBe("too_many_files");
    expect(filesRefusal([sized(20 * 1024 * 1024), sized(20 * 1024 * 1024), sized(1)])).toBe("batch_too_large");
  });

  it("routes a refusal to the pane the request belonged to", () => {
    const { events } = capture();
    submitAttachments("pane-9", [{ name: "shot.png", bytes: new Uint8Array([1]) }], true, false);
    const stage = events[0]!.payload.request_id as string;
    receiveAttachmentRefusal({ request_id: stage, reason: "too_large" });
    expect(useShellStore.getState().attachmentRefusal).toEqual({ pane_id: "pane-9", reason: "too_large" });
  });

  it("stages a name the daemon will accept, else a neutral one", () => {
    expect(safeName("shot.png")).toBe("shot.png");
    expect(safeName("")).toBe("attachment.bin");
    expect(safeName("..")).toBe("attachment.bin");
    expect(safeName("a/b")).toBe("attachment.bin");
    expect(safeName("a\\b")).toBe("attachment.bin");
    expect(safeName("a".repeat(97))).toBe("attachment.bin");
    expect(safeName("a\u0085b")).toBe("attachment.bin");
  });

  it("turns a reason into one line", () => {
    expect(refusalText("too_large", en)).toContain("20 MiB");
    expect(refusalText("batch_too_large", en)).toContain("40 MiB");
    expect(refusalText("too_many_files", en)).toContain("1 and 8");
    expect(refusalText("staging_full", en)).toContain("staged");
    expect(refusalText("size_mismatch", en)).toContain("changed");
    expect(refusalText("wat", en)).toBe("The attachment was refused.");
  });

  it("states the daemon's caps in the chosen language", () => {
    expect(refusalText("too_large", ko)).toBe("파일 하나가 첨부 한도 20 MiB를 초과합니다.");
    expect(refusalText("too_many_files", ko)).toBe("일반 파일을 1개에서 8개까지 선택하세요.");
  });
});
