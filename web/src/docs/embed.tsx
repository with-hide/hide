// The parts the user docs draw: the product's own components over invented English values, one part per
// `?part=` so a docs page frames the picture it needs and loads only that picture's code. Nothing here reads a
// daemon; the build is `docs-embed.config.ts`, which `hided` never serves.
import "./clock";
import { lazy, StrictMode, Suspense, useMemo } from "react";
import { createRoot } from "react-dom/client";
import "../index.css";
import { requireInterfaceLanguage } from "../i18n/locale";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import { readPartQuery } from "./parts";
import { RENDERERS } from "./renderers";

function Embed({ params }: { params: URLSearchParams }) {
  useFollowInterfaceLanguage(requireInterfaceLanguage(params.get("lang") ?? "en"));
  const query = readPartQuery(params);
  const Part = useMemo(() => lazy(RENDERERS[query.part]), [query.part]);
  return <Suspense fallback={null}><Part state={query.state} /></Suspense>;
}

const params = new URLSearchParams(window.location.search);
// The theme is the page's, fixed before the first paint so no part draws in the wrong one first.
const { theme } = readPartQuery(params);
document.documentElement.classList.toggle("dark", theme === "dark");
document.documentElement.classList.toggle("light", theme === "light");
const container = document.getElementById("root");
if (!container) throw new Error("root missing");
createRoot(container).render(<StrictMode><Embed params={params} /></StrictMode>);
