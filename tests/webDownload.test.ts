import { describe, it, expect, afterEach } from "bun:test";
import * as fs from "../src/lib/fs";

// Firefox and Safari start a download after click() returns. The download was
// silently lost when the blob URL was revoked on the next line, or when the
// anchor was never attached to the document.
describe("web file download", () => {
  const realCreate = URL.createObjectURL;
  const realRevoke = URL.revokeObjectURL;
  const realClick = HTMLAnchorElement.prototype.click;

  afterEach(() => {
    URL.createObjectURL = realCreate;
    URL.revokeObjectURL = realRevoke;
    HTMLAnchorElement.prototype.click = realClick;
  });

  it("attaches the link while clicking it and keeps the URL alive afterwards", async () => {
    const revoked: string[] = [];
    const seen: { attached?: boolean; download?: string; href?: string } = {};
    const captured: { blob?: Blob } = {};
    URL.createObjectURL = ((b: Blob) => {
      captured.blob = b;
      return "blob:zyplus-test";
    }) as typeof URL.createObjectURL;
    URL.revokeObjectURL = ((u: string) => void revoked.push(u)) as typeof URL.revokeObjectURL;
    HTMLAnchorElement.prototype.click = function (this: HTMLAnchorElement) {
      Object.assign(seen, { attached: this.isConnected, download: this.download, href: this.href });
    };

    await fs.saveFileAs("note.md", "# héllo");

    expect(seen).toEqual({ attached: true, download: "note.md", href: "blob:zyplus-test" });
    expect(document.querySelector("a[download]")).toBeNull(); // cleaned up
    expect(revoked).toEqual([]); // still alive: the browser may not have fetched it yet
    expect(captured.blob!.type).toBe("text/markdown;charset=utf-8");
    expect(await captured.blob!.text()).toBe("# héllo");
  });

  it("reports a failed export instead of throwing into a menu handler", async () => {
    const alerts: string[] = [];
    const realAlert = window.alert;
    window.alert = ((m: string) => void alerts.push(m)) as typeof window.alert;
    URL.createObjectURL = (() => {
      throw new Error("quota");
    }) as typeof URL.createObjectURL;
    try {
      await fs.saveFileAs("note.md", "x");
    } finally {
      window.alert = realAlert;
    }
    expect(alerts).toHaveLength(1);
    expect(alerts[0]).toContain("quota");
  });

  it("swaps an extension but keeps a dotfile's name", () => {
    expect(fs.withExtension("notes.txt", ".md")).toBe("notes.md");
    expect(fs.withExtension("a.b.md", ".pdf")).toBe("a.b.pdf");
    expect(fs.withExtension("README", ".md")).toBe("README.md");
    expect(fs.withExtension(".gitignore", ".md")).toBe(".gitignore.md");
  });
});
