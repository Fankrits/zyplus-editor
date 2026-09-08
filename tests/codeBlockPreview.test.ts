import { describe, it, expect } from "bun:test";
import { renderCodeBlockPreview } from "../src/components/Editor/codeBlockPreview";

/** Resolves with whatever the renderer hands to applyPreview. */
function preview(language: string, content: string) {
  return new Promise<null | string | HTMLElement>((resolve) => {
    const immediate = renderCodeBlockPreview(language, content, resolve);
    if (immediate !== undefined) resolve(immediate ?? null);
  });
}

describe("renderCodeBlockPreview", () => {
  it("renders a latex block through KaTeX", async () => {
    expect(await preview("latex", "x^2")).toContain("katex");
  });

  it("matches a language alias, case and padding aside", async () => {
    expect(await preview("  TeX ", "x^2")).toContain("katex");
  });

  it("leaves a language nothing renders alone", async () => {
    expect(await preview("python", "print(1)")).toBeNull();
  });

  it("leaves an empty block alone", async () => {
    expect(await preview("latex", "   \n ")).toBeNull();
  });
});
