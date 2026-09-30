import { describe, it, expect, beforeEach } from "bun:test";
import { ASSET_REF, EXTENSION_CATALOG, assetUrl, formatBytes, getManifestById } from "../src/extensions/catalog";
import { extensionManager, hashOf } from "../src/extensions/extensionManager";
import { isExtensionInstalledLocally, saveExtensionFiles } from "../src/extensions/loader";

describe("Extension System", () => {
  beforeEach(async () => {
    localStorage.clear();
    // Reset states
    await extensionManager.uninstallAndRemove("mermaid");
    await extensionManager.uninstallAndRemove("katex");
    await extensionManager.uninstallAndRemove("json");
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

  // Every asset URL is fetched at runtime from a real ref. Pointing them at a
  // branch that had been deleted made Mermaid and KaTeX un-installable in every
  // shipped build, and nothing failed until a user clicked Install.
  it("points every bundle at a ref that exists in the repository", () => {
    // A release tag when built by Vite; `main` for builds that predate pinning.
    expect(ASSET_REF).toMatch(/^(main|v\d+\.\d+\.\d+.*)$/);
    const urls = EXTENSION_CATALOG.flatMap((e) => [e.downloadUrl, e.cssUrl].filter(Boolean));
    expect(urls.length).toBeGreaterThanOrEqual(3);
    for (const url of urls) {
      // Dev serves these locally; a release build must resolve to the repo.
      if (url!.startsWith("/")) continue;
      expect(url).toStartWith(
        `https://raw.githubusercontent.com/Fankrits/zyplus-editor/${ASSET_REF}/extensions/dist/`,
      );
    }
  });

  // The web build's package.json version is never bumped outside a release run,
  // so the tag it named had no bundles and every browser install was a 404. The
  // browser installs from its own deploy; only the desktop app uses the tag.
  it("serves bundles from the deploy on the web and from the release tag on desktop", () => {
    expect(assetUrl("json.js", false, false)).toBe("/extensions-dist/json.js");
    expect(assetUrl("json.js", true, true)).toBe("/extensions-dist/json.js");
    expect(assetUrl("json.js", true, false)).toBe(
      `https://raw.githubusercontent.com/Fankrits/zyplus-editor/${ASSET_REF}/extensions/dist/json.js`,
    );
  });

  it("does not save a bundle whose stylesheet failed to download", async () => {
    const manifest = getManifestById("json")!;
    const originalFetch = globalThis.fetch;
    globalThis.fetch = (async (url: string | URL | Request) => {
      const isCss = String(url).endsWith(".css");
      return new Response(isCss ? "not found" : "export default {}", { status: isCss ? 404 : 200 });
    }) as unknown as typeof fetch;
    const realHash = manifest.sha256;
    manifest.sha256 = await hashOf("export default {}");
    try {
      await expect(extensionManager.downloadAndInstall("json")).rejects.toThrow(/styles/);
      expect(extensionManager.getState("json")?.status).toBe("error");
      expect(await isExtensionInstalledLocally("json")).toBe(false);
      expect(extensionManager.getEnabledIds()).not.toContain("json");
    } finally {
      globalThis.fetch = originalFetch;
      manifest.sha256 = realHash;
    }
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

    // The catalog pins the real bundle's hash; this test ships a stand-in.
    const manifest = getManifestById("mermaid")!;
    const realHash = manifest.sha256;
    manifest.sha256 = await hashOf(mockBundleCode);

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
      manifest.sha256 = realHash;
    }
  });

  it("hands .json files to the JSON editor only once it is enabled", () => {
    const json = getManifestById("json");
    expect(json?.fileExtensions).toEqual([".json"]);
    expect(json?.cssUrl).toBeDefined();

    expect(extensionManager.getFileEditorId("/notes/config.json")).toBeNull();

    localStorage.setItem("zyplus:enabled-extensions", JSON.stringify(["json"]));
    expect(extensionManager.getFileEditorId("/notes/config.json")).toBe("json");
    expect(extensionManager.getFileEditorId("C:\\notes\\CONFIG.JSON")).toBe("json");
    expect(extensionManager.getFileEditorId("/notes/todo.md")).toBeNull();
    expect(extensionManager.getFileEditorId("/notes/data.json.bak")).toBeNull();
    // It owns files, not code blocks: a ```json fence in a note keeps rendering as plain code.
    expect(extensionManager.getPreviewRenderer("json")).toBeNull();
  });

  // The bundle is fetched from the network and then run with the app's full
  // privileges; anything but the exact file this build shipped with is refused.
  it("refuses a bundle whose hash does not match, and keeps nothing of it", async () => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = (async () => new Response("export default () => ({ id: 'katex' })")) as unknown as typeof fetch;
    try {
      await expect(extensionManager.downloadAndInstall("katex")).rejects.toThrow("integrity");
      expect(extensionManager.getState("katex")?.status).toBe("error");
      expect(await isExtensionInstalledLocally("katex")).toBe(false);
    } finally {
      globalThis.fetch = originalFetch;
    }
  });

  // Mermaid's real bundle is megabytes; evaluating it at every launch cost time
  // and memory for sessions that never show a diagram.
  it("evaluates an installed bundle only when a code block first needs it", async () => {
    const g = globalThis as unknown as { __mermaidEvals?: number };
    g.__mermaidEvals = 0;
    await saveExtensionFiles(
      "mermaid",
      `globalThis.__mermaidEvals++;
       export default () => ({ id: "mermaid", renderCodeBlockPreview: () => "<svg>lazy</svg>" });`,
    );
    localStorage.setItem("zyplus:enabled-extensions", JSON.stringify(["mermaid"]));
    // As an install records itself; without this the copy reads as stale and is re-downloaded.
    localStorage.setItem(
      "zyplus:installed-extension-versions",
      JSON.stringify({ mermaid: getManifestById("mermaid")!.sha256 }),
    );

    await extensionManager.initialize();
    expect(extensionManager.getState("mermaid")?.status).toBe("installed");
    expect(g.__mermaidEvals).toBe(0);

    const preview = await new Promise((resolve) => {
      const result = extensionManager.getPreviewRenderer("mermaid")?.("mermaid", "graph TD", resolve);
      expect(result).toBeUndefined();
    });
    expect(preview).toBe("<svg>lazy</svg>");
    expect(g.__mermaidEvals).toBe(1);

    // Loaded once, then rendered synchronously.
    expect(extensionManager.getPreviewRenderer("mermaid")?.("mermaid", "graph TD", () => {})).toBe("<svg>lazy</svg>");
    expect(g.__mermaidEvals).toBe(1);
  });

  // An install is a copy. One made by an older build would run forever, so nothing
  // that changed in the bundle (a fix, a restyle) could ever reach anyone who had
  // already installed it.
  describe("an installed copy from an older build", () => {
    const bundle = (marker: string) =>
      `export default () => ({ id: "mermaid", marker: "${marker}" });`;
    const marker = (rt: unknown) => (rt as { marker: string }).marker;
    const installed = async () => {
      const { readTextFile, joinPath } = await import("../src/lib/fs");
      const { getExtensionDir } = await import("../src/extensions/loader");
      return readTextFile(await joinPath(await getExtensionDir("mermaid"), "index.js"));
    };

    it("is replaced by the bundle this build pins, before it runs", async () => {
      // Installed by an older build: no record of which bundle it came from.
      await saveExtensionFiles("mermaid", bundle("old"));
      localStorage.setItem("zyplus:enabled-extensions", JSON.stringify(["mermaid"]));

      const manifest = getManifestById("mermaid")!;
      const realHash = manifest.sha256;
      manifest.sha256 = await hashOf(bundle("new"));
      const originalFetch = globalThis.fetch;
      globalThis.fetch = (async () => new Response(bundle("new"))) as unknown as typeof fetch;
      try {
        expect(marker(await extensionManager.loadRuntime("mermaid"))).toBe("new");
        expect(await installed()).toBe(bundle("new"));
      } finally {
        globalThis.fetch = originalFetch;
        manifest.sha256 = realHash;
      }
    });

    it("keeps working offline, using the copy it has", async () => {
      await saveExtensionFiles("mermaid", bundle("old"));
      localStorage.setItem("zyplus:enabled-extensions", JSON.stringify(["mermaid"]));

      const originalFetch = globalThis.fetch;
      globalThis.fetch = (async () => {
        throw new TypeError("Failed to fetch");
      }) as unknown as typeof fetch;
      const originalWarn = console.warn;
      console.warn = () => {};
      try {
        expect(marker(await extensionManager.loadRuntime("mermaid"))).toBe("old");
        expect(await installed()).toBe(bundle("old"));
      } finally {
        globalThis.fetch = originalFetch;
        console.warn = originalWarn;
      }
    });

    it("is not replaced by a download that fails its integrity check", async () => {
      await saveExtensionFiles("mermaid", bundle("old"));
      localStorage.setItem("zyplus:enabled-extensions", JSON.stringify(["mermaid"]));

      const originalFetch = globalThis.fetch;
      globalThis.fetch = (async () => new Response(bundle("tampered"))) as unknown as typeof fetch;
      const originalWarn = console.warn;
      console.warn = () => {};
      try {
        expect(marker(await extensionManager.loadRuntime("mermaid"))).toBe("old");
        expect(await installed()).toBe(bundle("old"));
      } finally {
        globalThis.fetch = originalFetch;
        console.warn = originalWarn;
      }
    });
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
