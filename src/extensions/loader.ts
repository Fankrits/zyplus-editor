import { isTauri } from "@tauri-apps/api/core";
import { appDataDir } from "@tauri-apps/api/path";
import {
  deletePath,
  ensureFolder,
  joinPath,
  pathExists,
  readTextFile,
  writeTextFile,
} from "../lib/fs";
import type { ExtensionContext, ExtensionManifest, ExtensionRuntime } from "./types";
import { isDarkTheme } from "../lib/theme";

/**
 * Bundles live in the app data directory on desktop. The web build stores them
 * through the same filesystem layer as notes, in a dot-folder no project shows.
 */
export async function getExtensionsRootDir(): Promise<string> {
  return isTauri() ? joinPath(await appDataDir(), "extensions") : "/.extensions";
}

export async function getExtensionDir(id: string): Promise<string> {
  return joinPath(await getExtensionsRootDir(), id);
}

export async function isExtensionInstalledLocally(id: string): Promise<boolean> {
  try {
    return await pathExists(await joinPath(await getExtensionDir(id), "index.js"));
  } catch {
    return false;
  }
}

export async function saveExtensionFiles(
  id: string,
  jsContent: string,
  cssContent?: string,
): Promise<void> {
  const dir = await getExtensionDir(id);
  await ensureFolder(dir);
  await writeTextFile(await joinPath(dir, "index.js"), jsContent);
  if (cssContent) await writeTextFile(await joinPath(dir, "style.css"), cssContent);
}

export async function deleteExtensionFiles(id: string): Promise<void> {
  try {
    const dir = await getExtensionDir(id);
    if (await pathExists(dir)) await deletePath(dir, true);
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
  const dir = await getExtensionDir(manifest.id);
  const jsCode = await readTextFile(await joinPath(dir, "index.js"));

  const cssPath = await joinPath(dir, "style.css");
  const cssCode = manifest.cssUrl && (await pathExists(cssPath)) ? await readTextFile(cssPath) : null;

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
