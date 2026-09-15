import { isTauri } from "@tauri-apps/api/core";
import { appDataDir, join } from "@tauri-apps/api/path";
import { exists, readTextFile, writeTextFile, remove, mkdir } from "@tauri-apps/plugin-fs";
import type { ExtensionContext, ExtensionManifest, ExtensionRuntime } from "./types";
import { isDarkTheme } from "../lib/theme";

// In-memory fallback cache for headless testing or non-Tauri environments
const mockExtensionFiles = new Map<string, string>();

export async function getExtensionsRootDir(): Promise<string> {
  if (!isTauri()) {
    return "/mock-app-data/extensions";
  }
  const appData = await appDataDir();
  return await join(appData, "extensions");
}

export async function getExtensionDir(id: string): Promise<string> {
  const root = await getExtensionsRootDir();
  if (!isTauri()) {
    return `${root}/${id}`;
  }
  return await join(root, id);
}

export async function isExtensionInstalledLocally(id: string): Promise<boolean> {
  if (!isTauri()) {
    return mockExtensionFiles.has(`${id}/index.js`);
  }
  try {
    const dir = await getExtensionDir(id);
    const bundlePath = await join(dir, "index.js");
    return await exists(bundlePath);
  } catch {
    return false;
  }
}

export async function saveExtensionFiles(
  id: string,
  jsContent: string,
  cssContent?: string,
): Promise<void> {
  if (!isTauri()) {
    mockExtensionFiles.set(`${id}/index.js`, jsContent);
    if (cssContent) mockExtensionFiles.set(`${id}/style.css`, cssContent);
    return;
  }

  const dir = await getExtensionDir(id);
  const dirExists = await exists(dir);
  if (!dirExists) {
    await mkdir(dir, { recursive: true });
  }

  const bundlePath = await join(dir, "index.js");
  await writeTextFile(bundlePath, jsContent);

  if (cssContent) {
    const cssPath = await join(dir, "style.css");
    await writeTextFile(cssPath, cssContent);
  }
}

export async function deleteExtensionFiles(id: string): Promise<void> {
  if (!isTauri()) {
    mockExtensionFiles.delete(`${id}/index.js`);
    mockExtensionFiles.delete(`${id}/style.css`);
    return;
  }

  try {
    const dir = await getExtensionDir(id);
    const dirExists = await exists(dir);
    if (dirExists) {
      await remove(dir, { recursive: true });
    }
  } catch (err) {
    console.warn(`Failed to delete extension files for ${id}:`, err);
  }
}

export function injectExtensionCss(id: string, cssContent: string): void {
  removeExtensionCss(id);
  if (typeof document === "undefined") return;
  const styleEl = document.createElement("style");
  styleEl.id = `zyplus-ext-${id}`;
  styleEl.textContent = cssContent;
  document.head.appendChild(styleEl);
}

export function removeExtensionCss(id: string): void {
  if (typeof document === "undefined") return;
  const existing = document.getElementById(`zyplus-ext-${id}`);
  if (existing) {
    existing.remove();
  }
}

export async function loadExtensionModule(manifest: ExtensionManifest): Promise<ExtensionRuntime> {
  let jsCode: string;
  let cssCode: string | null = null;

  if (!isTauri()) {
    const cached = mockExtensionFiles.get(`${manifest.id}/index.js`);
    if (!cached) throw new Error(`Extension ${manifest.id} is not installed locally`);
    jsCode = cached;
    cssCode = mockExtensionFiles.get(`${manifest.id}/style.css`) ?? null;
  } else {
    const dir = await getExtensionDir(manifest.id);
    const bundlePath = await join(dir, "index.js");
    jsCode = await readTextFile(bundlePath);

    if (manifest.cssUrl) {
      const cssPath = await join(dir, "style.css");
      if (await exists(cssPath)) {
        cssCode = await readTextFile(cssPath);
      }
    }
  }

  if (cssCode) {
    injectExtensionCss(manifest.id, cssCode);
  }

  const blob = new Blob([jsCode], { type: "text/javascript" });
  const blobUrl = URL.createObjectURL(blob);

  try {
    const mod = await import(/* @vite-ignore */ blobUrl);
    const runtime: ExtensionRuntime =
      typeof mod.default === "function"
        ? mod.default()
        : mod.default || mod;

    if (runtime && runtime.activate) {
      const context: ExtensionContext = {
        isDarkTheme,
      };
      await runtime.activate(context);
    }

    return runtime;
  } finally {
    URL.revokeObjectURL(blobUrl);
  }
}
