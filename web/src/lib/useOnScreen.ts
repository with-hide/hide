import { useEffect, useRef, useState } from "react";

/**
 * Whether the element is on screen in a window that shows, so a moving
 * picture's loop runs only then: off screen or in a hidden window its
 * animations are paused where they are (the caller marks it `data-playing`).
 */
export function useOnScreen<T extends Element>(): [React.RefObject<T | null>, boolean] {
  const ref = useRef<T>(null);
  const [seen, setSeen] = useState(false);
  const [shown, setShown] = useState(() => typeof document === "undefined" || document.visibilityState !== "hidden");
  useEffect(() => {
    const node = ref.current;
    if (!node || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver((entries) => setSeen(entries.some((entry) => entry.isIntersecting)));
    observer.observe(node);
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    const update = () => setShown(document.visibilityState !== "hidden");
    document.addEventListener("visibilitychange", update);
    return () => document.removeEventListener("visibilitychange", update);
  }, []);
  return [ref, seen && shown];
}
