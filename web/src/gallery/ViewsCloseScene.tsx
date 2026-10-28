// The real question a session close asks when the Views it closes hold
// unsaved work (PRD session-files D-14), over invented file names.
// `count=1` shows the one-file form.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { AlertDialog, AlertDialogContent } from "../components/ui/alert-dialog";
import { TooltipProvider } from "../components/ui/tooltip";
import { requireInterfaceLanguage } from "../i18n/locale";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import { ViewsCloseSheet } from "../Overlays";

const ROWS = [
  { id: "d12", label: "prd.md" },
  { id: "d13", label: "menu.tsx" },
];

export function ViewsCloseScene({ params }: { params: URLSearchParams }) {
  useFollowInterfaceLanguage(requireInterfaceLanguage(params.get("lang") ?? "ko"));
  const theme = params.get("theme") === "light" ? "light" : "dark";
  const rows = params.get("count") === "1" ? ROWS.slice(0, 1) : ROWS;
  const actions = useMemo(() => createActions(() => true), []);
  useLayoutEffect(() => {
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
  }, [theme]);
  return (
    <TooltipProvider>
      <div className="h-full bg-background" data-gallery-scene="views-close">
        <AlertDialog open>
          <AlertDialogContent data-confirm-views-close="true">
            <ViewsCloseSheet actions={actions} rows={rows} />
          </AlertDialogContent>
        </AlertDialog>
      </div>
    </TooltipProvider>
  );
}
