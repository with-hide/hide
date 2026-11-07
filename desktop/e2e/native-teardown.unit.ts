// The report of a native spec that failed names which side failed and keeps the first
// failure's own text, so a flaky run's issue says what broke.
import { expect, test } from "vitest";
import { finishNative, inStep } from "./native-teardown";

const message = async (run: Promise<void>): Promise<string> => run.then(() => "", (error: unknown) => (error as Error).message);

test("a spec whose body and teardown passed ends without an error, every step run in order", async () => {
  const ran: string[] = [];
  await finishNative([], [["close the app", () => ran.push("app")], ["stop the stack", () => ran.push("stack")]]);
  expect(ran).toEqual(["app", "stack"]);
});

test("the body's failure stays the test's failure and a failed step is named beside it", async () => {
  const body = new Error("expect(locator).toContainText(expected) failed\n\nLocator: row");
  const ran: string[] = [];
  const ended = finishNative([body], [
    ["close the app", () => { throw new Error("app did not exit"); }],
    ["stop the stack", () => ran.push("stack")],
  ]);
  await expect(ended).rejects.toBe(body);
  expect(body.message).toContain("expect(locator).toContainText(expected) failed");
  expect(body.message).toContain("cleanup after it also failed: close the app: app did not exit");
  expect(ran).toEqual(["stack"]);
});

test("a spec whose body passed names the step that failed", async () => {
  const text = await message(finishNative([], [["close the app", () => { throw new Error("app did not exit"); }], ["stop the stack", () => { throw new Error("daemon still running"); }]]));
  expect(text.startsWith("close the app: app did not exit")).toBe(true);
  expect(text).toContain("stop the stack: daemon still running");
});

test("a failure the body recorded and went on from is named by its step and ranks after the first", async () => {
  const first = new Error("first");
  await expect(finishNative([first, inStep("native window capture", new Error("no window"))], [])).rejects.toBe(first);
  expect(first.message).toContain("native window capture: no window");
});

test("a thrown value that is no Error is still reported with its text", async () => {
  expect(await message(finishNative(["plain"], [["stop the stack", () => { throw new Error("late"); }]]))).toContain("plain");
});
