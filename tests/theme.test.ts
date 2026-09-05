import { describe, it, expect, beforeEach } from "bun:test";

const store: Record<string, string> = {};
const g = globalThis as Record<string, unknown>;
g.localStorage = {
  getItem: (k: string) => store[k] ?? null,
  setItem: (k: string, v: string) => void (store[k] = v),
  removeItem: (k: string) => void delete store[k],
  clear: () => Object.keys(store).forEach((k) => delete store[k]),
};
g.document = { documentElement: { dataset: {} as Record<string, string>, style: {} } };
g.window = g;

const { applyTheme, getTheme, setTheme } = await import("../src/lib/theme");

function mockSystemDark(isDark: boolean) {
  g.matchMedia = () => ({ matches: isDark, addEventListener: () => {} });
}

describe("theme", () => {
  beforeEach(() => {
    localStorage.clear();
    mockSystemDark(false);
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

  it("resolves system against the OS preference", () => {
    mockSystemDark(true);
    applyTheme("system");
    expect(document.documentElement.dataset.theme).toBe("dark");
    mockSystemDark(false);
    applyTheme("system");
    expect(document.documentElement.dataset.theme).toBe("light");
  });
});
