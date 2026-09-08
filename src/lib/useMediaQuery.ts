import { useSyncExternalStore } from "react";

export function useMediaQuery(query: string): boolean {
  return useSyncExternalStore(
    (callback) => {
      if (typeof window === "undefined" || !window.matchMedia) return () => {};
      // `change` fires on every crossing of the breakpoint, resize or not.
      const mql = window.matchMedia(query);
      mql.addEventListener("change", callback);
      return () => mql.removeEventListener("change", callback);
    },
    () => {
      if (typeof window === "undefined" || !window.matchMedia) return false;
      return window.matchMedia(query).matches;
    },
    () => false
  );
}

export function useIsDesktop(): boolean {
  return useMediaQuery("(min-width: 768px)");
}
