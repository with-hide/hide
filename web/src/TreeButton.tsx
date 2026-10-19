import { FolderIcon } from "lucide-react";
import type { Actions } from "./actions";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { commandLabel } from "./shortcutLabels";
import { frontCheckout } from "./snapshot";
import { useShellStore } from "./store";
import { workspaceViewOf } from "./workspace";

/**
 * The files tool's persistent Explorer switch. With no View, the tree is
 * the remaining surface and cannot be turned off.
 */
export function TreeButton({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const name = useShellStore((s) => {
    const checkout = frontCheckout(s.rest);
    return checkout ? (checkout.branch ?? checkout.label) : null;
  });
  const on = useShellStore((s) => workspaceViewOf(s.rest)?.tree === true);
  const treeOnly = useShellStore((s) => (workspaceViewOf(s.rest)?.layout?.display_count ?? 0) === 0);
  const chord = useShellStore((s) => commandLabel("toggle_file_tree", s.rest?.ui_state));
  if (!name) return null;
  return (
    <Hint label={t("panes.lane.tree")} shortcut={chord}>
      <button
        type="button"
        aria-pressed={on}
        disabled={on && treeOnly}
        aria-label={`${t("panes.lane.tree")} · ${name}`}
        className={`flex h-(--size-control-sm) shrink-0 items-center gap-xxs rounded-sm px-xs text-caption outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring ${on ? "bg-secondary text-foreground" : "text-subtle-foreground"}`}
        data-lane-tree-toggle={on ? "on" : "off"}
        onPointerDown={(event) => event.stopPropagation()}
        onClick={() => actions.toggleFileTree()}
      >
        <FolderIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
        <span>{t("panes.lane.tree")}</span>
      </button>
    </Hint>
  );
}
