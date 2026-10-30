/**
 * Where the View menu, Help and Settings stand: at the foot of the device
 * rail, stacked and opening to the right, or, while the rail is hidden, at the
 * sidebar footer's right edge, opening upwards.
 */
export type MenuPlace = "footer" | "rail";

export const hintSide = (place: MenuPlace) => (place === "rail" ? "right" : undefined);
export const menuSide = (place: MenuPlace) => (place === "rail" ? "right" : "top");
