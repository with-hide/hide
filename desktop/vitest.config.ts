import { defineConfig } from "vitest/config";

export default defineConfig({
  test: { include: ["src/**/*.test.ts", "scripts/**/*.test.mjs", "e2e/fixture-cleanup.unit.ts", "e2e/windows-processes.unit.ts"] },
});
