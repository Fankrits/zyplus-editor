import { useSyncExternalStore } from "react";

export function useMediaQuery(query: string): boolean {
  return useSyncExternalStore(
    (callback) => {
      if (typeof window === "undefined" || !window.matchMedia) return () => {};
      const mql = window.matchMedia(query);
      mql.addEventListener("change", callback);
      window.addEventListener("resize", callback);
      return () => {
        mql.removeEventListener("change", callback);
        window.removeEventListener("resize", callback);
      };
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
