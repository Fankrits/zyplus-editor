import { EXTENSION_CATALOG, getManifestById } from "./catalog";
import {
  deleteExtensionFiles,
  isExtensionInstalledLocally,
  loadExtensionModule,
  removeExtensionCss,
  saveExtensionFiles,
} from "./loader";
import type { ExtensionRuntime, ExtensionState } from "./types";

const ENABLED_STORAGE_KEY = "zyplus:enabled-extensions";

type Listener = () => void;

class ExtensionManager {
  private states: Map<string, ExtensionState> = new Map();
  private activeRuntimes: Map<string, ExtensionRuntime> = new Map();
  private listeners: Set<Listener> = new Set();
  private isInitialized = false;

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

  public async initialize(): Promise<void> {
    if (this.isInitialized) return;
    this.isInitialized = true;

    const enabledIds = new Set(this.getEnabledIds());

    for (const manifest of EXTENSION_CATALOG) {
      const isInstalled = await isExtensionInstalledLocally(manifest.id);
      if (isInstalled && enabledIds.has(manifest.id)) {
        try {
          const runtime = await loadExtensionModule(manifest);
          this.activeRuntimes.set(manifest.id, runtime);
          this.states.set(manifest.id, {
            manifest,
            status: "installed",
          });
        } catch (err) {
          console.error(`Failed to activate extension ${manifest.id}:`, err);
          this.states.set(manifest.id, {
            manifest,
            status: "error",
            errorMessage: err instanceof Error ? err.message : String(err),
          });
        }
      } else {
        // Not installed or not enabled
        this.states.set(manifest.id, {
          manifest,
          status: "uninstalled",
        });
      }
    }

    this.notify();
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

      // 4. Dynamically load runtime
      const runtime = await loadExtensionModule(manifest);
      this.activeRuntimes.set(id, runtime);

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
    for (const [id, runtime] of this.activeRuntimes.entries()) {
      const manifest = getManifestById(id);
      if (manifest && manifest.languages.includes(normalizedLang) && runtime.renderCodeBlockPreview) {
        return runtime.renderCodeBlockPreview.bind(runtime);
      }
    }
    return null;
  }
}

export const extensionManager = new ExtensionManager();
