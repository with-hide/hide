// The phone app's entry (PRD mobile-companion D-02, D-16, B38): follow the
// phone's light or dark setting and the operator's color theme
// (hide-color-theme D-12), register the service worker that caches the static
// shell and shows pushes, and start the one socket.

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "../index.css";
import "./mobile.css";
import { App } from "./App";
import { openDetail, start } from "./connection";
import { PhoneLanguageBoundary } from "./language";
import { openKey } from "./protocol";
import { hostedPhone } from "./relayRuntime";
import { patch, usePhone } from "./store";
import { PALETTE_HIDE_CLASS, readColorTheme } from "../theme";

const scheme = window.matchMedia("(prefers-color-scheme: dark)");
const followScheme = () => document.documentElement.classList.toggle("dark", scheme.matches);
followScheme();
scheme.addEventListener("change", followScheme);

const followColorTheme = (value: string | null) =>
  document.documentElement.classList.toggle(PALETTE_HIDE_CLASS, readColorTheme(value).choice === "hide");
followColorTheme(usePhone.getState().colorTheme);
usePhone.subscribe((state, previous) => {
  if (state.colorTheme !== previous.colorTheme) followColorTheme(state.colorTheme);
});

if ("serviceWorker" in navigator) {
  void navigator.serviceWorker.register(hostedPhone() ? "/sw.js" : "/m/sw.js", { scope: hostedPhone() ? "/" : "/m/" }).catch(() => {
    if (hostedPhone()) patch({ relayError: "loadError" });
  });
  // A tapped notification while the app is open: the worker names the agent (B32).
  navigator.serviceWorker.addEventListener("message", (event: MessageEvent<{ type?: string; tag?: string }>) => {
    const key = event.data?.type === "open" && event.data.tag ? openKey(event.data.tag) : null;
    if (key) openDetail(key);
  });
}

start();

const container = document.getElementById("root");
if (!container) throw new Error("root missing");
createRoot(container).render(
  <StrictMode>
    <PhoneLanguageBoundary />
    <App />
  </StrictMode>,
);
