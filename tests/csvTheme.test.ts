import { describe, it, expect } from "bun:test";
import fs from "node:fs";

const read = (p: string) => fs.readFileSync(new URL(p, import.meta.url), "utf-8");

describe("CSV editor theme", () => {
  // The built stylesheet is Tabulator's own structural CSS (its markup needs this to lay
  // out as a grid at all) with our HeroUI recoloring appended on top, mirroring how
  // build-extensions.ts assembles it.
  const tabulatorBase = read("../node_modules/tabulator-tables/dist/css/tabulator.min.css");
  const source = read("../src/extensions/bundles/csv.css");

  it("is what the built stylesheet contains", () => {
    expect(read("../extensions/dist/csv.css")).toBe(tabulatorBase + "\n" + source);
  });

  // Everything in OUR file is a HeroUI token: a literal would keep its color in every
  // palette. Tabulator's own base legitimately has literal colors (its default theme);
  // those are harmless since every one we care about is overridden with `!important`,
  // so this only scans the file we author, not the vendor stylesheet it sits on top of.
  it("contains no color literals of its own", () => {
    const withoutComments = source.replace(/\/\*[\s\S]*?\*\//g, "");
    expect(withoutComments.match(/#[0-9a-f]{3,8}\b|\brgba?\(|\bhsla?\(/gi)).toBeNull();
  });
});
