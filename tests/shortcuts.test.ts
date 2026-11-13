import { describe, it, expect } from "bun:test";
import { matchShortcut, matchesCombo, formatCombo } from "../src/lib/shortcuts";

// The platform is passed in rather than faked on `navigator`: a global set at
// module scope only lands if this file happens to load before anything else
// that reads it, which made the result depend on test order.
const MAC = true;
const PC = false;

function key(init: Partial<KeyboardEvent>): KeyboardEvent {
  return { code: "", key: "", metaKey: false, ctrlKey: false, shiftKey: false, altKey: false, ...init } as KeyboardEvent;
}

describe("shortcuts", () => {
  it("matches a plain mod combo", () => {
    expect(matchShortcut(key({ code: "KeyS", metaKey: true }), MAC)).toBe("save");
  });

  // The bracket keys are named "]" / "[" by keyOf; an alias spelled "bracketright" never matched.
  it("switches tabs with Cmd+Shift+] and Cmd+Shift+[", () => {
    expect(matchShortcut(key({ code: "BracketRight", metaKey: true, shiftKey: true }), MAC)).toBe("next-tab");
    expect(matchShortcut(key({ code: "BracketLeft", ctrlKey: true, shiftKey: true }), PC)).toBe("prev-tab");
  });

  it("distinguishes shifted combos", () => {
    expect(matchShortcut(key({ code: "KeyS", metaKey: true, shiftKey: true }), MAC)).toBe("export-md");
  });

  // On macOS Alt rewrites `key` ("ƒ"), so matching must go through `code`.
  it("matches alt combos by physical key", () => {
    expect(matchShortcut(key({ code: "KeyF", key: "ƒ", metaKey: true, altKey: true }), MAC)).toBe("replace");
  });

  it("ignores the bare key and the wrong platform modifier", () => {
    expect(matchShortcut(key({ code: "KeyS", key: "s" }), MAC)).toBeNull();
    expect(matchShortcut(key({ code: "KeyS", ctrlKey: true }), MAC)).toBeNull();
  });

  it("matches punctuation and arrow aliases", () => {
    expect(matchesCombo(key({ code: "Comma", metaKey: true }), "mod+,", MAC)).toBe(true);
    expect(matchesCombo(key({ code: "ArrowRight", metaKey: true, altKey: true }), "mod+alt+arrowright", MAC)).toBe(
      true,
    );
  });

  it("zooms the text with Cmd/Ctrl and plus, minus and zero, from either keyboard row", () => {
    expect(matchShortcut(key({ code: "Equal", key: "=", metaKey: true }), MAC)).toBe("zoom-in");
    // A typed "+" on a US layout is Shift+=.
    expect(matchShortcut(key({ code: "Equal", key: "+", ctrlKey: true, shiftKey: true }), PC)).toBe("zoom-in");
    expect(matchShortcut(key({ code: "NumpadAdd", key: "+", ctrlKey: true }), PC)).toBe("zoom-in");
    expect(matchShortcut(key({ code: "Minus", key: "-", ctrlKey: true }), PC)).toBe("zoom-out");
    expect(matchShortcut(key({ code: "NumpadSubtract", key: "-", metaKey: true }), MAC)).toBe("zoom-out");
    expect(matchShortcut(key({ code: "Digit0", key: "0", ctrlKey: true }), PC)).toBe("zoom-reset");
    expect(matchShortcut(key({ code: "Numpad0", key: "0", metaKey: true }), MAC)).toBe("zoom-reset");
    // The bare keys type text.
    expect(matchShortcut(key({ code: "Minus", key: "-" }), PC)).toBeNull();
  });

  it("shows the zoom-in combo as a plus", () => {
    expect(formatCombo("mod+=", PC)).toEqual(["Ctrl", "+"]);
    expect(formatCombo("mod+=", MAC)).toEqual(["⌘", "+"]);
  });

  it("formats combos for display", () => {
    expect(formatCombo("mod+shift+s", MAC)).toEqual(["⌘", "⇧", "S"]);
  });

  it("uses Ctrl-based bindings and labels off macOS", () => {
    expect(matchShortcut(key({ code: "KeyS", ctrlKey: true }), PC)).toBe("save");
    // ⌘ must not trigger anything on a platform where it is the Windows key.
    expect(matchShortcut(key({ code: "KeyS", metaKey: true }), PC)).toBeNull();
    expect(formatCombo("mod+shift+s", PC)).toEqual(["Ctrl", "Shift", "S"]);
  });
});
