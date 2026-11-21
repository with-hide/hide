import { neighbourBox, type Edge } from "./areaLayout";

/** Only the panes drawn beside this keyboard owner, in this tab and Agent area. */
export function adjacentPane(paneId: string, direction: Edge): string | null {
  const pane = document.querySelector(`[data-pane-view="${CSS.escape(paneId)}"]`);
  const canvas = pane?.closest<HTMLElement>("[data-canvas]");
  if (!canvas || canvas.dataset.zoomed === "true") return null;
  const boxes = Array.from(canvas.querySelectorAll<HTMLElement>("[data-layout-pane]"))
    .map((element) => ({ id: element.dataset.layoutPane!, rect: element.getBoundingClientRect() }))
    .filter(({ rect }) => rect.width > 0 && rect.height > 0);
  // CSS grid, percentage positions and page zoom round each DOMRect separately.
  // Treat edges within one physical display pixel as the same drawn edge.
  return neighbourBox(boxes, paneId, direction, 1 / window.devicePixelRatio);
}
