import { EXTENSION_CATALOG, getManifestById } from "./catalog";
import {
  deleteExtensionFiles,
  isExtensionInstalledLocally,
  loadExtensionModule,
  removeExtensionCss,
  saveExtensionFiles,
} from "./loader";
import type { ExtensionManifest, ExtensionRuntime, ExtensionState } from "./types";

const ENABLED_STORAGE_KEY = "zyplus:enabled-extensions";

type Listener = () => void;

class ExtensionManager {
  private states: Map<string, ExtensionState> = new Map();
  private activeRuntimes: Map<string, ExtensionRuntime> = new Map();
  private listeners: Set<Listener> = new Set();
  private isInitialized = false;
  private loading: Map<string, Promise<ExtensionRuntime>> = new Map();

  constructor() {
    for (const manifest of EXTENSION_CATALOG) {
      this.states.set(manifest.id, {
        manifest,
        status: "uninstalled",
      });
    }
  }

  public subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  private notify() {
    for (const listener of this.listeners) {
      listener();
    }
  }

  public getStates(): ExtensionState[] {
    return Array.from(this.states.values());
  }

  public getState(id: string): ExtensionState | undefined {
    return this.states.get(id);
  }

  public getEnabledIds(): string[] {
    try {
      const raw = localStorage.getItem(ENABLED_STORAGE_KEY);
      if (!raw) return [];
      const parsed = JSON.parse(raw);
      return Array.isArray(parsed) ? parsed : [];
    } catch {
      return [];
    }
  }

  private saveEnabledIds(ids: string[]) {
    try {
      localStorage.setItem(ENABLED_STORAGE_KEY, JSON.stringify(ids));
    } catch (err) {
      console.warn("Failed to persist enabled extensions to localStorage:", err);
    }
  }

  /**
   * Only works out what is installed, for the settings list. Evaluating a bundle
   * (Mermaid's is several megabytes) waits until a code block first asks for it,
   * so a session that never shows a diagram never pays for one.
   */
  public async initialize(): Promise<void> {
    if (this.isInitialized) return;
    this.isInitialized = true;

    const enabledIds = new Set(this.getEnabledIds());
    await Promise.all(EXTENSION_CATALOG.map(async (manifest) => {
      if (this.states.get(manifest.id)?.status !== "uninstalled") return;
      const isInstalled = enabledIds.has(manifest.id) && (await isExtensionInstalledLocally(manifest.id));
      if (isInstalled) this.states.set(manifest.id, { manifest, status: "installed" });
    }));

    this.notify();
  }

  /** Evaluates an extension's bundle once; concurrent callers share the load. */
  private load(manifest: ExtensionManifest): Promise<ExtensionRuntime> {
    const { id } = manifest;
    let pending = this.loading.get(id);
    if (pending) return pending;
    pending = loadExtensionModule(manifest).then(
      (runtime) => {
        // Uninstalled while it was loading.
        if (this.loading.get(id) === pending) this.activeRuntimes.set(id, runtime);
        return runtime;
      },
      (err) => {
        if (this.loading.get(id) === pending) {
          this.loading.delete(id);
          console.error(`Failed to activate extension ${id}:`, err);
          this.states.set(id, {
            manifest,
            status: "error",
            errorMessage: err instanceof Error ? err.message : String(err),
          });
          this.notify();
        }
        throw err;
      },
    );
    this.loading.set(id, pending);
    return pending;
  }

  public async downloadAndInstall(id: string): Promise<void> {
    const manifest = getManifestById(id);
    if (!manifest) throw new Error(`Unknown extension: ${id}`);

    this.states.set(id, {
      manifest,
      status: "downloading",
      downloadProgress: 10,
    });
    this.notify();

    try {
      // 1. Fetch JavaScript bundle
      const jsRes = await fetch(manifest.downloadUrl);
      if (!jsRes.ok) {
        throw new Error(`Failed to download ${manifest.name}: HTTP ${jsRes.status} ${jsRes.statusText}`);
      }
      const jsCode = await jsRes.text();

      this.states.set(id, {
        manifest,
        status: "downloading",
        downloadProgress: 70,
      });
      this.notify();

      // 2. Fetch CSS bundle if specified
      let cssCode: string | undefined;
      if (manifest.cssUrl) {
        const cssRes = await fetch(manifest.cssUrl);
        if (cssRes.ok) {
          cssCode = await cssRes.text();
        }
      }

      this.states.set(id, {
        manifest,
        status: "downloading",
        downloadProgress: 90,
      });
      this.notify();

      // 3. Save to local disk under $APP_DATA/extensions/<id>/
      await saveExtensionFiles(id, jsCode, cssCode);

      // 4. Dynamically load runtime (fresh, in case an older copy was loaded)
      this.loading.delete(id);
      await this.load(manifest);

      // 5. Persist enabled state
      const current = new Set(this.getEnabledIds());
      current.add(id);
      this.saveEnabledIds(Array.from(current));

      this.states.set(id, {
        manifest,
        status: "installed",
      });
      this.notify();
    } catch (err) {
      console.error(`Failed to install extension ${id}:`, err);
      // Clean up any partially written files
      await deleteExtensionFiles(id);
      this.states.set(id, {
        manifest,
        status: "error",
        errorMessage: err instanceof Error ? err.message : String(err),
      });
      this.notify();
      throw err;
    }
  }

  public async uninstallAndRemove(id: string): Promise<void> {
    const manifest = getManifestById(id);
    if (!manifest) return;

    // 1. Deactivate runtime
    const activeRuntime = this.activeRuntimes.get(id);
    if (activeRuntime?.deactivate) {
      try {
        await activeRuntime.deactivate();
      } catch (err) {
        console.warn(`Error deactivating extension ${id}:`, err);
      }
    }
    this.activeRuntimes.delete(id);
    this.loading.delete(id);
    removeExtensionCss(id);

    // 2. Completely remove files from user's disk
    await deleteExtensionFiles(id);

    // 3. Remove from enabled persistence
    const current = new Set(this.getEnabledIds());
    current.delete(id);
    this.saveEnabledIds(Array.from(current));

    // 4. Update status to uninstalled
    this.states.set(id, {
      manifest,
      status: "uninstalled",
    });
    this.notify();
  }

  public getPreviewRenderer(
    language: string,
  ): ExtensionRuntime["renderCodeBlockPreview"] | null {
    const normalizedLang = language.trim().toLowerCase();
    const manifest = EXTENSION_CATALOG.find((m) => m.languages.includes(normalizedLang));
    if (!manifest) return null;
    const runtime = this.activeRuntimes.get(manifest.id);
    if (runtime) return runtime.renderCodeBlockPreview?.bind(runtime) ?? null;
    // The enabled list is synchronous, so a block rendered before initialize()
    // has run still gets its preview instead of none until it is next edited.
    if (!this.getEnabledIds().includes(manifest.id)) return null;
    return (lang, content, applyPreview) => {
      this.load(manifest).then(
        (rt) => {
          const result = rt.renderCodeBlockPreview?.(lang, content, applyPreview);
          if (result !== undefined) applyPreview(result);
        },
        () => applyPreview(null),
      );
      // Undefined tells Milkdown the preview arrives later through applyPreview.
      return undefined;
    };
  }
}

export const extensionManager = new ExtensionManager();
