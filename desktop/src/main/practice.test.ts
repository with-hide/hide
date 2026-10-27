import { describe, expect, it } from "vitest";
import { practiceBounds, practiceUrl } from "./practice";

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

describe("practiceBounds", () => {
  it("opens smaller than the main window, against its lower right, so the main window shows above and to the left", () => {
    expect(practiceBounds({ x: 100, y: 50, width: 1400, height: 900 })).toEqual({ x: 310, y: 185, width: 1190, height: 765 });
  });

  it("never opens below the smallest window size", () => {
    expect(practiceBounds({ x: 0, y: 0, width: 720, height: 480 })).toEqual({ x: 0, y: 0, width: 720, height: 480 });
  });

  it("takes the default size when there is no main window", () => {
    expect(practiceBounds(undefined)).toEqual({ width: 1280, height: 800 });
  });
});
