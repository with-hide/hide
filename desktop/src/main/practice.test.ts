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
  const screen = { x: 0, y: 0, width: 1920, height: 1080 };

  it("opens smaller than the main window, against its lower right, so the main window shows above and to the left", () => {
    expect(practiceBounds({ x: 100, y: 50, width: 1800, height: 900 }, screen)).toEqual({ x: 370, y: 185, width: 1530, height: 765 });
  });

  it("opens wide enough to show the lane beside the agents, keeping to the screen", () => {
    expect(practiceBounds({ x: 100, y: 50, width: 1400, height: 900 }, screen)).toEqual({ x: 220, y: 185, width: 1280, height: 765 });
    expect(practiceBounds({ x: 0, y: 0, width: 720, height: 480 }, screen)).toEqual({ x: 0, y: 0, width: 1280, height: 480 });
  });

  it("takes the whole screen's width when the screen is narrower than that", () => {
    expect(practiceBounds({ x: 0, y: 0, width: 1100, height: 700 }, { x: 0, y: 0, width: 1152, height: 720 })).toEqual({ x: 0, y: 105, width: 1152, height: 595 });
  });

  it("takes the default size when there is no main window", () => {
    expect(practiceBounds(undefined, screen)).toEqual({ width: 1280, height: 800 });
  });
});
