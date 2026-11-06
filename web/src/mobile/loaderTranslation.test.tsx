// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useLoaderTranslation } from "./loaderTranslation";
import { DEFAULT_SETTINGS, choosePhoneSettings } from "./settings";

function Probe() {
  const { t } = useLoaderTranslation();
  return <p>{t("mobile.relay.devices")}</p>;
}

afterEach(() => {
  choosePhoneSettings(DEFAULT_SETTINGS);
  vi.restoreAllMocks();
  localStorage.clear();
});

describe("the start page's language", () => {
  it("is this phone's own choice, else the phone's language", async () => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    vi.spyOn(window.navigator, "language", "get").mockReturnValue("ko-KR");
    choosePhoneSettings({ language: "ja" });
    const host = document.createElement("div");
    const root = createRoot(host);
    await act(async () => root.render(<Probe />));
    expect(host.textContent).toBe("デバイス");
    expect(document.documentElement.lang).toBe("ja");
    choosePhoneSettings({ language: null });
    await act(async () => root.render(<Probe />));
    await act(async () => window.dispatchEvent(new Event("languagechange")));
    expect(host.textContent).toBe("기기 고르기");
    await act(async () => root.unmount());
  });
});
