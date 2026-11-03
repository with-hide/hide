import { describe, expect, it } from "vitest";
import cases from "../../contracts/fixtures/diagnostic-redaction.json";
import { redact } from "./redact";

describe("diagnostic privacy policy shared with Rust", () => {
  for (const { name, input, expected } of cases) {
    it(name, () => expect(redact(typeof input === "string" ? input : input.join(""))).toBe(expected));
  }
});
