import { expect, it } from "vitest";
import { takeProgramCopy } from "./clipboard";

function recorder() {
  const written: [string, string][] = [];
  const logged: string[] = [];
  const write = (text: string, what: string) => {
    written.push([text, what]);
    return Promise.resolve(true);
  };
  return { written, logged, write, diagnostic: (message: string) => logged.push(message) };
}

// with-hide/hide-private#659: the window in front writes a program's copy,
// its text read as UTF-8, as a terminal writes it for the client in front.
it("the window in front writes a program's copy as its text", async () => {
  const seen = recorder();
  const text = "복사 – copied\n";
  const encoded = btoa(String.fromCharCode(...new TextEncoder().encode(text)));
  expect(await takeProgramCopy({ pane_id: "w1:p1", text_base64: encoded }, true, seen.write, seen.diagnostic)).toBe(true);
  expect(seen.written).toEqual([[text, "pane w1:p1 copy"]]);
  expect(seen.logged).toEqual([]);
});

// Every window is handed the copy; one behind leaves the clipboard alone and
// says so in the log only.
it("a window behind leaves the clipboard alone", async () => {
  const seen = recorder();
  expect(await takeProgramCopy({ pane_id: "w1:p1", text_base64: "eA==" }, false, seen.write, seen.diagnostic)).toBe(false);
  expect(seen.written).toEqual([]);
  expect(seen.logged).toHaveLength(1);
});
