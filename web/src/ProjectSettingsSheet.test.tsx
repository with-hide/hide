// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterAll, expect, it, vi } from "vitest";
import { ProjectSettingsScene } from "./gallery/ProjectSettingsScene";
import { clientI18n } from "./i18n/translator";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

const browserLayout = vi.hoisted(() => {
  const canvas = HTMLCanvasElement.prototype.getContext;
  const scroll = Element.prototype.scrollIntoView;
  HTMLCanvasElement.prototype.getContext = () => null;
  // jsdom has no layout. Radix scrolls the selected option into its viewport.
  Element.prototype.scrollIntoView = () => {};
  return { restore: () => {
    HTMLCanvasElement.prototype.getContext = canvas;
    Element.prototype.scrollIntoView = scroll;
  } };
});
afterAll(() => browserLayout.restore());

it("keeps an acknowledged change settled when another window replaces its receipt", async () => {
  const savedShell = useShellStore.getState();
  const savedUi = useUiStore.getState();
  const language = clientI18n.language;
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => {
      await clientI18n.changeLanguage("en");
      root.render(<ProjectSettingsScene params={new URLSearchParams("lang=en")} />);
    });
    const control = () => document.querySelector<HTMLButtonElement>('[role="combobox"][aria-label="Issue source"]')!;
    await act(async () => control().dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true })));
    const local = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find((option) => option.textContent === "Local")!;
    expect(local).toBeDefined();
    await act(async () => local.click());
    expect(document.querySelector("[data-project-settings]")?.textContent).toContain("Saved");
    await act(async () => useShellStore.setState((shell) => {
      const record = shell.rest!.ui_state!.project_settings!.example!;
      return { rest: { ...shell.rest, ui_state: { ...shell.rest!.ui_state, project_settings_writes: {
        example: { device_id: record.device_id, root: record.root, request_id: "another-window", state: "saved", error: null, patch: { new_work: "worktree" } },
      } } } };
    }));
    expect(control().disabled).toBe(false);
    expect(document.querySelector("[data-project-settings]")?.textContent).not.toContain("Saving");
  } finally {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(savedShell, true);
    useUiStore.setState(savedUi, true);
    await clientI18n.changeLanguage(language);
    vi.unstubAllGlobals();
  }
});

/** Renders the scene in `state` and hands the test the dialog; restores the stores after. */
async function withScene(query: string, check: (sheet: () => HTMLElement) => Promise<void>) {
  const savedShell = useShellStore.getState();
  const savedUi = useUiStore.getState();
  const language = clientI18n.language;
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => {
      await clientI18n.changeLanguage("en");
      root.render(<ProjectSettingsScene params={new URLSearchParams(query)} />);
    });
    await check(() => document.querySelector<HTMLElement>("[data-project-settings]")!);
  } finally {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(savedShell, true);
    useUiStore.setState(savedUi, true);
    await clientI18n.changeLanguage(language);
    vi.unstubAllGlobals();
  }
}

const guard = (part: string) => document.querySelector<HTMLElement>(`[data-project-guard="${part}"]`)!;
const saved = () => Object.values(useShellStore.getState().rest?.ui_state?.project_settings ?? {})[0];

it("draws four protections where the unsupported rows were, and a change sends the whole set (B1, B2, B3)", async () => {
  await withScene("lang=en&state=guard-off", async (sheet) => {
    expect(sheet().textContent).not.toContain("Unsupported");
    const switches = [...sheet().querySelectorAll<HTMLButtonElement>('[data-project-guardrails] [role="switch"]')];
    expect(switches.map((control) => control.getAttribute("aria-label"))).toEqual(["Guard working folders", "Guard the base branch", "Block risky Git commands", "Guard files"]);
    expect(switches.every((control) => control.getAttribute("aria-checked") === "false")).toBe(true);
    // The merge checkbox and the path list show only under their switch.
    expect(guard("agent_merge")).toBeNull();
    expect(document.querySelector("[data-project-guard-path]")).toBeNull();
    await act(async () => switches[1]!.click());
    expect(saved()?.version).toBe(2);
    expect(saved()?.guardrails).toMatchObject({ base_branch: true, agent_merge: false, files: false });
    expect(guard("base_branch").textContent).toContain("Saved");
    await act(async () => guard("agent_merge").querySelector<HTMLButtonElement>('[role="checkbox"]')!.click());
    expect(saved()?.guardrails).toMatchObject({ base_branch: true, agent_merge: true });
  });
});

it("adds and removes protected paths and says why a path is refused in place (B15, B16)", async () => {
  await withScene("lang=en&state=guard-off", async (sheet) => {
    await act(async () => sheet().querySelector<HTMLButtonElement>('[role="switch"][aria-label="Guard files"]')!.click());
    const paths = () => [...document.querySelectorAll("[data-project-guard-path]")].map((chip) => chip.getAttribute("data-project-guard-path"));
    expect(paths()).toEqual([".env", ".env.*", "*.pem", "*.key", "~/.ssh/"]);
    const add = async (path: string) => {
      await act(async () => [...guard("files").querySelectorAll("button")].find((button) => button.textContent === "Add path")?.click());
      const input = guard("files").querySelector<HTMLInputElement>("input")!;
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, path);
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
      await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    };
    await add(".aws/");
    expect(paths()).toContain(".aws/");
    expect(saved()?.guardrails?.paths).toContain(".aws/");
    await add(".env");
    expect(guard("files").textContent).toContain("That path is already on the list.");
    expect(saved()?.guardrails?.paths.filter((path) => path === ".env")).toHaveLength(1);
    const input = () => guard("files").querySelector<HTMLInputElement>("input");
    // Escape leaves the field and keeps the dialog: the shell's Escape runs the innermost layer, which is the field's.
    await act(async () => useUiStore.getState().escapeLayers.at(-1)!());
    expect(input()).toBeNull();
    expect(sheet()).not.toBeNull();
    expect(document.activeElement?.textContent).toBe("Add path");
    await act(async () => sheet().querySelector<HTMLButtonElement>('[aria-label="Remove *.pem"]')!.click());
    expect(saved()?.guardrails?.paths).not.toContain("*.pem");
    expect(document.activeElement?.textContent).toBe("Add path");
    // A pattern only the core can judge: its refusal shows under the field, which keeps what was typed, with
    // nothing to retry.
    await add("[z-a]");
    expect(input()?.value).toBe("[z-a]");
    expect(document.activeElement).toBe(input());
    expect(guard("files").textContent).toContain("That isn't a valid pattern.");
    expect([...guard("files").querySelectorAll("button")].some((button) => button.textContent === "Try again")).toBe(false);
  });
});

it("keeps a folder's Git protections off and unavailable, and names the agents the guardrails miss (B17, B25)", async () => {
  await withScene("lang=en&state=guard-folder", async (sheet) => {
    const control = (label: string) => sheet().querySelector<HTMLButtonElement>(`[role="switch"][aria-label="${label}"]`)!;
    for (const label of ["Guard working folders", "Guard the base branch", "Block risky Git commands"]) expect(control(label).disabled).toBe(true);
    expect(control("Guard files").disabled).toBe(false);
    expect(document.querySelector("[data-project-guard-uncovered]")).toBeNull();
  });
  await withScene("lang=en&state=guard-uncovered", async () => {
    expect(document.querySelector("[data-project-guard-uncovered]")?.textContent).toContain("Guardrails don't apply to Codex, Grok.");
  });
});

it("turns the background fetch off and on for the project, on until the operator says otherwise (B9)", async () => {
  await withScene("lang=en&state=guard-off", async (sheet) => {
    const control = () => sheet().querySelector<HTMLButtonElement>('[role="switch"][aria-label="Check the remote in the background"]')!;
    expect(control().getAttribute("aria-checked")).toBe("true");
    await act(async () => control().click());
    expect(saved()?.auto_fetch).toBe(false);
    expect(control().getAttribute("aria-checked")).toBe("false");
    await act(async () => control().click());
    expect(saved()?.auto_fetch).toBe(true);
  });
});
