import { describe, it, expect } from "bun:test";
import fs from "node:fs";

const read = (p: string) => fs.readFileSync(new URL(p, import.meta.url), "utf-8");
const vars = (css: string) => new Set(css.match(/--[a-z0-9-]+(?=\s*:)/g));

// Geometry and behavior the library's own defaults are right for, or values that are already
// transparent/inherited. Anything else the library reads has to be themed in json.css.
const LIBRARY_DEFAULTS_OK = new Set([
  "--jse-color-picker-button-size",
  "--jse-contents-background-color",
  "--jse-contents-cursor",
  "--jse-contents-selected-cursor",
  "--jse-context-menu-pointer-size",
  "--jse-font-size-text-mode-search",
  "--jse-indent-size",
  "--jse-input-background-readonly",
  "--jse-line-height",
  "--jse-padding",
  "--jse-panel-button-background",
  "--jse-svelte-select-font-size",
  "--jse-svelte-select-multi-select-padding",
  "--jse-svelte-select-padding",
]);

describe("JSON editor theme", () => {
  const source = read("../src/extensions/bundles/json.css");

  // A variable the library adds and this file misses silently falls back to the
  // library's own palette: a blue toolbar in a Catppuccin window.
  it("restates every variable the library's own theme sets", () => {
    const library = vars(read("../node_modules/vanilla-jsoneditor/themes/jse-theme-dark.css"));
    const mine = vars(source);
    expect([...library].filter((v) => !mine.has(v))).toEqual([]);
  });

  // The app installs extensions/dist/json.css; editing the source and forgetting
  // `bun run build:extensions` would ship yesterday's look.
  it("is what the built stylesheet contains", () => {
    expect(read("../extensions/dist/json.css")).toBe(source);
  });

  // The first test only covers what the library's dark theme sets. Others (the warning bar's
  // #ffde5c, the info bar's #4f91ff, a disabled button's #9d9d9d) are hard-coded fallbacks that
  // no theme file mentions and that nothing on screen shows until such a bar or button appears.
  it("themes every variable the library reads, or lists it as fine as it is", () => {
    const library = read("../node_modules/vanilla-jsoneditor/index.js");
    const read_ = new Set([...library.matchAll(/var\((--jse-[a-z0-9-]+)/g)].map((m) => m[1]));
    const mine = vars(source);
    expect([...read_].filter((v) => !mine.has(v) && !LIBRARY_DEFAULTS_OK.has(v)).sort()).toEqual([]);
  });

  // Everything here is a HeroUI token: a literal would keep its color in every palette.
  it("contains no color literals of its own", () => {
    const withoutComments = source.replace(/\/\*[\s\S]*?\*\//g, "");
    expect(withoutComments.match(/#[0-9a-f]{3,8}\b|\brgba?\(|\bhsla?\(/gi)).toBeNull();
  });
});

