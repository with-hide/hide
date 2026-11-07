import { afterEach, describe, expect, it, vi } from "vitest";
import { inBackground } from "./inBackground";

const settled = () => new Promise<void>((resolve) => setImmediate(resolve));

describe("inBackground", () => {
  const rejections: unknown[] = [];
  const record = (reason: unknown) => { rejections.push(reason); };
  process.on("unhandledRejection", record);
  afterEach(() => { rejections.length = 0; });

  it("hands the response to the caller", async () => {
    const answered = vi.fn();
    const failed = vi.fn();
    inBackground(() => Promise.resolve(new Response("ok")), answered, failed);
    await settled();
    expect(answered).toHaveBeenCalledOnce();
    expect(failed).not.toHaveBeenCalled();
  });

  it("reports a request that throws instead of returning a promise, as the first fetch of a process can", async () => {
    const failed = vi.fn();
    const thrown = new TypeError("Cannot read properties of undefined (reading 'markResourceTiming')");
    inBackground(() => { throw thrown; }, vi.fn(), failed);
    await settled();
    expect(failed).toHaveBeenCalledWith(thrown);
    expect(rejections).toEqual([]);
  });

  it("reports a rejected request and a response the caller cannot read", async () => {
    const failed = vi.fn();
    inBackground(() => Promise.reject(new Error("refused")), vi.fn(), failed);
    inBackground(() => Promise.resolve(new Response("ok")), () => { throw new Error("unreadable"); }, failed);
    await settled();
    expect(failed.mock.calls.map(([error]) => (error as Error).message)).toEqual(["refused", "unreadable"]);
    expect(rejections).toEqual([]);
  });
});
