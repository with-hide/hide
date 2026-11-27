import { expect, it } from "vitest";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { installWindowScreen, windowScreen } from "./windowScreen";

it("reads the screen the window shows: this machine's own, or another computer's in its tile", () => {
  useUiStore.setState({ remoteScreen: null });
  expect(windowScreen()).toBe("own");
  useUiStore.setState({ remoteScreen: "mini" });
  expect(windowScreen()).toBe("computer");
  useUiStore.setState({ remoteScreen: null });
});

// The daemon holds nothing of a connection it has not met, so each new live connection is told once.
it("tells at once and again each time the connection is live again", () => {
  useShellStore.setState({ connection: "live" } as never);
  let told = 0;
  const remove = installWindowScreen(() => (told += 1));
  useShellStore.setState({ connection: "connecting" } as never);
  useShellStore.setState({ connection: "live" } as never);
  useShellStore.setState({ connection: "live", rest: null } as never);
  remove();
  useShellStore.setState({ connection: "connecting" } as never);
  useShellStore.setState({ connection: "live" } as never);
  expect(told).toBe(2);
});
