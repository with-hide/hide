import { describe, expect, it } from "vitest";
import { wakeRequest } from "./hideAiWake";

describe("wakeRequest", () => {
  const daemon = { url: "http://127.0.0.1:4810/#token=abc123", origin: "http://127.0.0.1:4810", port: 4810, pid: 7 };

  it("posts the cause to the attached core with its token", () => {
    const request = wakeRequest(daemon, "unlock");
    expect(request?.url).toBe("http://127.0.0.1:4810/hide-ai-wake");
    expect(request?.init.method).toBe("POST");
    expect(request?.init.headers).toEqual({ Authorization: "Bearer abc123", "Content-Type": "application/json" });
    expect(JSON.parse(String(request?.init.body))).toEqual({ cause: "unlock" });
  });

  it("sends nothing when the core's address carries no token", () => {
    expect(wakeRequest({ ...daemon, url: "http://127.0.0.1:4810/" }, "app_open")).toBeNull();
  });
});
