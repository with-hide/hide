import { describe, expect, it } from "vitest";
import { practiceUrl } from "./practice";

describe("practiceUrl", () => {
  it("names the practice page on the daemon's origin with the language and this host", () => {
    const url = new URL(practiceUrl("http://127.0.0.1:53111", "ko", "darwin") as string);
    expect(url.origin).toBe("http://127.0.0.1:53111");
    expect(url.pathname).toBe("/");
    expect(Object.fromEntries(url.searchParams)).toEqual({ practice: "1", lang: "ko", host: "electron", platform: "darwin" });
    expect(url.hash).toBe("");
  });

  it("opens on the chapter asked for, and refuses a chapter id the core would not keep", () => {
    const url = new URL(practiceUrl("http://127.0.0.1:53111", "en", "win32", "overview") as string);
    expect(url.searchParams.get("chapter")).toBe("overview");
    expect(practiceUrl("http://127.0.0.1:53111", "en", "win32", "../x")).toEqual({ refused: "chapter" });
    expect(practiceUrl("http://127.0.0.1:53111", "en", "win32", 0)).toEqual({ refused: "chapter" });
  });

  it("refuses a language the shell does not offer", () => {
    expect(practiceUrl("http://127.0.0.1:53111", "ko&host=browser", "darwin")).toEqual({ refused: "language" });
    expect(practiceUrl("http://127.0.0.1:53111", 7, "darwin")).toEqual({ refused: "language" });
  });
});
