import { describe, it, expect, beforeEach, afterEach } from "bun:test";
import { applyTheme, getTheme, setTheme } from "../src/lib/theme";

// Only the OS preference is faked; the DOM and localStorage are the shared ones
// from tests/setup.ts. Stubbing `document` here instead would replace it for
// every test file loaded after this one — see that file's comment.
const g = globalThis as Record<string, unknown>;
const realMatchMedia = g.matchMedia;

function mockSystemDark(isDark: boolean) {
  g.matchMedia = () => ({ matches: isDark, addEventListener: () => {} });
}

describe("theme", () => {
  beforeEach(() => {
    localStorage.clear();
    delete document.documentElement.dataset.theme;
    delete document.documentElement.dataset.palette;
    mockSystemDark(false);
  });

  afterEach(() => {
    g.matchMedia = realMatchMedia;
  });

  it("defaults to system and ignores junk", () => {
    expect(getTheme()).toBe("system");
    localStorage.setItem("zyplus:theme", "neon");
    expect(getTheme()).toBe("system");
  });

  it("persists and applies an explicit theme", () => {
    setTheme("dark");
    expect(getTheme()).toBe("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("applies a palette on top of its base theme, and clears it again", () => {
    setTheme("mocha");
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(document.documentElement.dataset.palette).toBe("mocha");
    expect(localStorage.getItem("zyplus:theme-base")).toBe("dark");
    setTheme("latte");
    expect(document.documentElement.dataset.theme).toBe("light");
    setTheme("system");
    expect(document.documentElement.dataset.palette).toBeUndefined();
    expect(localStorage.getItem("zyplus:theme-base")).toBe("");
  });

  it("resolves system against the OS preference", () => {
    mockSystemDark(true);
    applyTheme("system");
    expect(document.documentElement.dataset.theme).toBe("dark");
    mockSystemDark(false);
    applyTheme("system");
    expect(document.documentElement.dataset.theme).toBe("light");
  });
});
