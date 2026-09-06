import { describe, it, expect, beforeEach } from "bun:test";
import { EXTENSION_CATALOG, formatBytes } from "../src/extensions/catalog";
import { extensionManager } from "../src/extensions/extensionManager";
import { isExtensionInstalledLocally } from "../src/extensions/loader";

class LocalStorageMock implements Storage {
  private store: Record<string, string> = {};

  get length(): number {
    return Object.keys(this.store).length;
  }

  clear(): void {
    this.store = {};
  }

  getItem(key: string): string | null {
    return Object.prototype.hasOwnProperty.call(this.store, key) ? this.store[key] : null;
  }

  setItem(key: string, value: string): void {
    this.store[key] = String(value);
  }

  removeItem(key: string): void {
    delete this.store[key];
  }

  key(index: number): string | null {
    const keys = Object.keys(this.store);
    return keys[index] ?? null;
  }
}

if (typeof globalThis.localStorage === "undefined") {
  globalThis.localStorage = new LocalStorageMock();
}

describe("Extension System", () => {
  beforeEach(async () => {
    localStorage.clear();
    // Reset states
    await extensionManager.uninstallAndRemove("mermaid");
    await extensionManager.uninstallAndRemove("katex");
  });

  it("should provide valid catalog definitions for Mermaid and KaTeX", () => {
    expect(EXTENSION_CATALOG.length).toBeGreaterThanOrEqual(2);

    const mermaid = EXTENSION_CATALOG.find((e) => e.id === "mermaid");
    expect(mermaid).toBeDefined();
    expect(mermaid?.languages).toContain("mermaid");
    expect(mermaid?.languages).toContain("flowchart");
    expect(mermaid?.sizeBytesEstimate).toBeGreaterThan(1_000_000);

    const katex = EXTENSION_CATALOG.find((e) => e.id === "katex");
    expect(katex).toBeDefined();
    expect(katex?.languages).toContain("latex");
    expect(katex?.languages).toContain("math");

    expect(formatBytes(1024)).toBe("1.0 KB");
    expect(formatBytes(3_500_000)).toBe("3.3 MB");
  });

  it("should have extensions uninstalled by default", () => {
    const states = extensionManager.getStates();
    const mermaidState = states.find((s) => s.manifest.id === "mermaid");
    expect(mermaidState?.status).toBe("uninstalled");
    expect(extensionManager.getPreviewRenderer("mermaid")).toBeNull();
  });

  it("should download, install, activate, and allow uninstalling an extension", async () => {
    // Mock global fetch for extension download
    const mockBundleCode = `
      export default function() {
        return {
          id: "mermaid",
          renderCodeBlockPreview(lang, content, apply) {
            apply("<svg>mock-mermaid</svg>");
          }
        };
      }
    `;

    const originalFetch = globalThis.fetch;
    globalThis.fetch = (async (_url: string | URL | Request) => {
      return new Response(mockBundleCode, {
        status: 200,
        headers: { "Content-Type": "application/javascript" },
      });
    }) as unknown as typeof fetch;

    try {
      // 1. Download & install
      await extensionManager.downloadAndInstall("mermaid");

      const installedState = extensionManager.getState("mermaid");
      expect(installedState?.status).toBe("installed");

      // Verify files exist locally
      const installedLocally = await isExtensionInstalledLocally("mermaid");
      expect(installedLocally).toBe(true);

      // Verify persistence
      expect(extensionManager.getEnabledIds()).toContain("mermaid");

      // Verify renderer is active
      const renderer = extensionManager.getPreviewRenderer("mermaid");
      expect(typeof renderer).toBe("function");

      let previewOutput: string | HTMLElement | null = null;
      renderer?.("mermaid", "flowchart TD\n A-->B", (output: string | HTMLElement | null) => {
        previewOutput = output;
      });
      expect(previewOutput as unknown as string).toBe("<svg>mock-mermaid</svg>");

      // 2. Uninstall & remove
      await extensionManager.uninstallAndRemove("mermaid");

      const uninstalledState = extensionManager.getState("mermaid");
      expect(uninstalledState?.status).toBe("uninstalled");

      // Verify files removed from disk/cache
      const stillInstalled = await isExtensionInstalledLocally("mermaid");
      expect(stillInstalled).toBe(false);

      // Verify removed from persistence
      expect(extensionManager.getEnabledIds()).not.toContain("mermaid");

      // Verify renderer deactivated
      expect(extensionManager.getPreviewRenderer("mermaid")).toBeNull();
    } finally {
      globalThis.fetch = originalFetch;
    }
  });

  it("should handle download failures cleanly without leaving corrupted state", async () => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = (async () => {
      return new Response("Not Found", { status: 404, statusText: "Not Found" });
    }) as unknown as typeof fetch;

    try {
      await expect(extensionManager.downloadAndInstall("katex")).rejects.toThrow();

      const state = extensionManager.getState("katex");
      expect(state?.status).toBe("error");
      expect(state?.errorMessage).toContain("404");

      // Should not be saved as enabled
      expect(extensionManager.getEnabledIds()).not.toContain("katex");
      // Files should not exist
      const installedLocally = await isExtensionInstalledLocally("katex");
      expect(installedLocally).toBe(false);
    } finally {
      globalThis.fetch = originalFetch;
    }
  });
});
