import { describe, it, expect, beforeEach, afterEach } from "bun:test";
import { renderHook, act } from "@testing-library/react";
import { useMediaQuery, useIsDesktop } from "../src/lib/useMediaQuery";

describe("useMediaQuery & useIsDesktop", () => {
  let listeners: ((e: MediaQueryListEvent) => void)[] = [];
  let matchesValue = false;
  const originalMatchMedia = window.matchMedia;

  beforeEach(() => {
    listeners = [];
    matchesValue = false;
    window.matchMedia = (query: string): MediaQueryList =>
      ({
        matches: query === "(min-width: 768px)" ? matchesValue : false,
        media: query,
        onchange: null,
        addListener: () => {},
        removeListener: () => {},
        addEventListener: (_event: string, handler: (e: MediaQueryListEvent) => void) => {
          listeners.push(handler);
        },
        removeEventListener: (_event: string, handler: (e: MediaQueryListEvent) => void) => {
          listeners = listeners.filter((h) => h !== handler);
        },
        dispatchEvent: () => true,
      }) as unknown as MediaQueryList;
  });

  afterEach(() => {
    window.matchMedia = originalMatchMedia;
  });

  it("evaluates initial match value for useIsDesktop", () => {
    matchesValue = true;
    const { result } = renderHook(() => useIsDesktop());
    expect(result.current).toBe(true);
  });

  it("evaluates false when initial match value does not match", () => {
    matchesValue = false;
    const { result } = renderHook(() => useIsDesktop());
    expect(result.current).toBe(false);
  });

  it("updates when media query change event fires", () => {
    matchesValue = false;
    const { result } = renderHook(() => useIsDesktop());
    expect(result.current).toBe(false);

    act(() => {
      matchesValue = true;
      listeners.forEach((l) => l({ matches: true } as MediaQueryListEvent));
    });

    expect(result.current).toBe(true);
  });

  it("removes event listener on unmount", () => {
    const { unmount } = renderHook(() => useMediaQuery("(min-width: 768px)"));
    expect(listeners.length).toBe(1);

    unmount();
    expect(listeners.length).toBe(0);
  });

  it("handles arbitrary custom queries with useMediaQuery", () => {
    let customMatch = false;
    window.matchMedia = (query: string): MediaQueryList =>
      ({
        matches: query === "(max-width: 480px)" ? customMatch : false,
        media: query,
        onchange: null,
        addListener: () => {},
        removeListener: () => {},
        addEventListener: (_event: string, handler: (e: MediaQueryListEvent) => void) => {
          listeners.push(handler);
        },
        removeEventListener: (_event: string, handler: (e: MediaQueryListEvent) => void) => {
          listeners = listeners.filter((h) => h !== handler);
        },
        dispatchEvent: () => true,
      }) as unknown as MediaQueryList;

    customMatch = true;
    const { result } = renderHook(() => useMediaQuery("(max-width: 480px)"));
    expect(result.current).toBe(true);
  });
});
