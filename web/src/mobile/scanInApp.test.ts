// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { scanInApp } from "./ScanQr";

const t = ((key: string) => key) as never;

function scannerRejects(code: string) {
  vi.stubGlobal("location", { ...window.location, protocol: "capacitor:" });
  vi.stubGlobal("Capacitor", { nativePromise: async () => { throw Object.assign(new Error(code), { code }); }, addListener: () => ({ remove: () => undefined }) });
}
beforeEach(() => vi.unstubAllGlobals());
afterEach(() => vi.unstubAllGlobals());

describe("the app's QR scanner failures", () => {
  it("says the camera is busy when another screen holds it, not that a permission is missing", async () => {
    scannerRejects("busy");
    const fail = vi.fn();
    await scanInApp(t, vi.fn(), fail);
    expect(fail).toHaveBeenCalledWith("cameraBusy");
  });

  it("keeps the camera message for a camera that cannot be used, and says nothing when the person closes it", async () => {
    const fail = vi.fn();
    scannerRejects("camera_unavailable");
    await scanInApp(t, vi.fn(), fail);
    expect(fail).toHaveBeenCalledWith("cameraError");
    fail.mockClear();
    scannerRejects("cancelled");
    await scanInApp(t, vi.fn(), fail);
    expect(fail).not.toHaveBeenCalled();
  });
});
