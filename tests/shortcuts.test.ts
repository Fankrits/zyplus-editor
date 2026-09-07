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
