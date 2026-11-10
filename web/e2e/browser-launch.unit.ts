import { expect, test } from "vitest";
import { disabledTestFeatures, testBrowserArgs } from "./browser-launch.cjs";

test("Windows test launches keep the browser defaults and custom features in one switch", () => {
  const args = testBrowserArgs(["--use-angle=swiftshader", "--disable-features=FixtureFeature", "--disable-features=PaintHolding"], "win32");
  const disabled = args.filter((arg) => arg.startsWith("--disable-features="));
  expect(disabled).toHaveLength(1);
  const features = disabled[0]!.slice("--disable-features=".length).split(",");
  // PaintHolding and Translate are required by the locked upstream defaults.
  expect(features).toEqual(expect.arrayContaining(["PaintHolding", "Translate", "FixtureFeature", "TcpPortRandomizationWin"]));
  expect(features.filter((feature) => feature === "PaintHolding")).toHaveLength(1);
  expect(args).toContain("--use-angle=swiftshader");
});

test("Unix test launches retain their supplied arguments", () => {
  const args = ["--disable-webgl", "--disable-features=FixtureFeature"];
  expect(testBrowserArgs(args, "darwin")).toEqual(args);
  expect(testBrowserArgs(args, "linux")).toEqual(args);
});

test("the Electron preload extends the features its loader installed without duplicating them", () => {
  expect(disabledTestFeatures("PaintHolding,TcpPortRandomizationWin,FixtureFeature", "win32")).toBe("PaintHolding,TcpPortRandomizationWin,FixtureFeature");
  expect(disabledTestFeatures("PaintHolding,FixtureFeature", "win32")).toBe("PaintHolding,FixtureFeature,TcpPortRandomizationWin");
  expect(disabledTestFeatures("PaintHolding", "darwin")).toBe("PaintHolding");
});
