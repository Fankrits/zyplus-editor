import { useCallback, useSyncExternalStore } from "react";

export function useMediaQuery(query: string): boolean {
  // Stable per query: an inline subscribe made React unsubscribe and
  // resubscribe on every render of every component using this.
  const subscribe = useCallback(
    (callback: () => void) => {
      if (typeof window === "undefined" || !window.matchMedia) return () => {};
      // `change` fires on every crossing of the breakpoint, resize or not.
      const mql = window.matchMedia(query);
      mql.addEventListener("change", callback);
      return () => mql.removeEventListener("change", callback);
    },
    [query],
  );
  const getSnapshot = useCallback(() => {
    if (typeof window === "undefined" || !window.matchMedia) return false;
    return window.matchMedia(query).matches;
  }, [query]);
  return useSyncExternalStore(subscribe, getSnapshot, () => false);
}

export function useIsDesktop(): boolean {
  return useMediaQuery("(min-width: 768px)");
}
