import { describe, expect, it } from "vitest";
import { fallbackScreen, fallbackScript, statusPageFile } from "./statusPage";

const WORDS = {
  lang: "en",
  connecting: "Starting Hide…",
  failed: "Can't connect to hided",
  retry: "Retry",
  cli_missing: "The hide command was not found.",
  start_failed: "hided could not start.",
  state_refused: "hided could not start because of this file.",
  core_newer_title: "Update this app",
};

describe("the status page file", () => {
  // `path.join` spells the result in the system's separator; the layout is what the test pins.
  const file = (dirname: string) => statusPageFile(dirname).replace(/\\/g, "/");

  it("is read beside the archive, never from inside it", () => {
    expect(file("/Applications/hide.app/Contents/Resources/app.asar/dist")).toBe("/Applications/hide.app/Contents/Resources/app.asar.unpacked/dist/static/status.html");
    expect(file("C:\\Program Files\\hide\\resources\\app.asar\\dist")).toBe("C:/Program Files/hide/resources/app.asar.unpacked/dist/static/status.html");
    expect(file("/Applications/hide.app/Contents/Resources/app.asar/dist")).not.toMatch(/app\.asar\/dist/);
  });

  it("stays where an unpackaged run keeps it, and does not unpack twice", () => {
    expect(file("/work/hide/desktop/dist")).toBe("/work/hide/desktop/dist/static/status.html");
    expect(file("/r/app.asar.unpacked/dist")).toBe("/r/app.asar.unpacked/dist/static/status.html");
  });
});

describe("the plain status screen", () => {
  it("says what the status page says for each failure", () => {
    expect(fallbackScreen(WORDS, { kind: "connecting" })).toMatchObject({ heading: null, reason: "Starting Hide…", reasonCode: null });
    expect(fallbackScreen(WORDS, { kind: "failed", reason: "cli_missing" })).toMatchObject({ heading: "Can't connect to hided", reason: "The hide command was not found.", reasonCode: "cli_missing", retry: "Retry" });
    // An unknown reason reads as a failed start, as the page does.
    expect(fallbackScreen(WORDS, { kind: "failed", reason: "from_the_future" }).reason).toBe("hided could not start.");
    expect(fallbackScreen(WORDS, { kind: "failed", reason: "core_newer", said: "The core is newer.", builds: "core 2 · this app 1" })).toMatchObject({
      heading: "Update this app",
      reason: "The core is newer.",
      lines: ["core 2 · this app 1"],
    });
  });

  it("fails loudly when the host passed no sentence for the reason", () => {
    expect(() => fallbackScreen({ lang: "en", retry: "Retry", failed: "x" }, { kind: "failed", reason: "no_response" })).toThrow("no_response");
  });

  it("draws every sentence as text and offers Retry as a fragment link", () => {
    const hostile = fallbackScreen(WORDS, { kind: "failed", reason: "state_refused", file: '/x/<img src=x onerror="boom()">' });
    const made: { tag: string; textContent: string; href?: string; attributes: Record<string, string> }[] = [];
    const element = (tag: string) => {
      const created = { tag, textContent: "", style: { cssText: "" }, dataset: {} as Record<string, string>, attributes: {} as Record<string, string>, id: "", href: undefined as string | undefined, setAttribute: (key: string, value: string) => (created.attributes[key] = value) };
      return created;
    };
    const document = {
      documentElement: { lang: "", style: { cssText: "" } },
      title: "",
      body: { style: { cssText: "" }, replaceChildren: () => (made.length = 0), append: (child: (typeof made)[number]) => made.push(child) },
      createElement: element,
    };
    new Function("document", `return ${fallbackScript(hostile, "#101010")}`)(document);
    expect(made.map((child) => child.textContent)).toEqual(["Can't connect to hided", "hided could not start because of this file.", '/x/<img src=x onerror="boom()">', "Retry"]);
    expect(made.at(-1)).toMatchObject({ tag: "a", href: "#retry", attributes: { role: "button" } });
    expect(document.documentElement.style.cssText).toContain("#101010");
  });
});
