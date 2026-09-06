import { describe, it, expect } from "bun:test";

const g = globalThis as Record<string, unknown>;
g.navigator = { userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)" };

const { matchShortcut, matchesCombo, formatCombo } = await import("../src/lib/shortcuts");

function key(init: Partial<KeyboardEvent>): KeyboardEvent {
  return { code: "", key: "", metaKey: false, ctrlKey: false, shiftKey: false, altKey: false, ...init } as KeyboardEvent;
}

describe("shortcuts", () => {
  it("matches a plain mod combo", () => {
    expect(matchShortcut(key({ code: "KeyS", metaKey: true }))).toBe("save");
  });

  it("distinguishes shifted combos", () => {
    expect(matchShortcut(key({ code: "KeyS", metaKey: true, shiftKey: true }))).toBe("export-md");
  });

  // On macOS Alt rewrites `key` ("ƒ"), so matching must go through `code`.
  it("matches alt combos by physical key", () => {
    expect(matchShortcut(key({ code: "KeyF", key: "ƒ", metaKey: true, altKey: true }))).toBe("replace");
  });

  it("ignores the bare key and the wrong platform modifier", () => {
    expect(matchShortcut(key({ code: "KeyS", key: "s" }))).toBeNull();
    expect(matchShortcut(key({ code: "KeyS", ctrlKey: true }))).toBeNull();
  });

  it("matches punctuation and arrow aliases", () => {
    expect(matchesCombo(key({ code: "Comma", metaKey: true }), "mod+,")).toBe(true);
    expect(matchesCombo(key({ code: "ArrowRight", metaKey: true, altKey: true }), "mod+alt+arrowright")).toBe(true);
  });

  it("formats combos for display", () => {
    expect(formatCombo("mod+shift+s")).toEqual(["⌘", "⇧", "S"]);
  });
});
