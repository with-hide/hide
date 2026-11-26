import { createContext, useContext, type ReactNode } from "react";

// A floating surface (menu, popover, select, tooltip) draws --popover, the
// ⌘K palette's step, the same as a modal's. Opened inside a modal it would
// melt into the modal, so a modal marks what it holds and a floating surface
// opened there sets --popover to --popover-nested, one step above the modal:
// a menu always stands one step above the surface it opened on. React context
// crosses the portal a floating surface renders through, which a CSS variable
// set on the modal would not.

const InModal = createContext(false);

/** Wraps a modal's content, so a floating surface opened from it draws one step above it. */
export function ModalSurface({ children }: { children: ReactNode }) {
  return <InModal.Provider value>{children}</InModal.Provider>;
}

/** The class a floating surface adds to its content: inside a modal, --popover becomes --popover-nested. */
export function useFloatingSurface(): string | undefined {
  return useContext(InModal) ? "[--popover:var(--popover-nested)]" : undefined;
}
