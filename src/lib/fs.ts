import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  readDir,
  readTextFile as readTextFileRaw,
  writeTextFile as writeTextFileRaw,
  mkdir,
  remove,
  rename as renameRaw,
  exists,
} from "@tauri-apps/plugin-fs";
import { join, dirname, basename } from "@tauri-apps/api/path";
import type { TreeNode } from "../state/workspaceStore";

const MARKDOWN_EXTENSIONS = [".md", ".markdown", ".txt"];

export function isMarkdownFile(name: string): boolean {
  const lower = name.toLowerCase();
  return MARKDOWN_EXTENSIONS.some((ext) => lower.endsWith(ext));
}

export async function openFolderDialog(): Promise<string | null> {
  const result = await openDialog({ directory: true });
  return typeof result === "string" ? result : null;
}

/** Reads a directory tree top to bottom, skipping dotfiles/dot-directories. */
export async function readDirRecursive(dirPath: string): Promise<TreeNode[]> {
  const entries = await readDir(dirPath);
  const nodes: TreeNode[] = [];
  for (const entry of entries) {
    if (entry.name.startsWith(".")) continue;
    const path = await join(dirPath, entry.name);
    if (entry.isDirectory) {
      nodes.push({
        id: path,
        name: entry.name,
        isFolder: true,
        children: await readDirRecursive(path),
      });
    } else {
      nodes.push({ id: path, name: entry.name, isFolder: false });
    }
  }
  nodes.sort((a, b) => {
    if (a.isFolder !== b.isFolder) return a.isFolder ? -1 : 1;
    return a.name.localeCompare(b.name);
  });
  return nodes;
}

export async function readTextFile(path: string): Promise<string> {
  return readTextFileRaw(path);
}

export async function writeTextFile(path: string, content: string): Promise<void> {
  await writeTextFileRaw(path, content);
}

export async function createFile(path: string): Promise<void> {
  if (await exists(path)) throw new Error(`"${path}" already exists`);
  await writeTextFileRaw(path, "");
}

export async function createFolder(path: string): Promise<void> {
  if (await exists(path)) throw new Error(`"${path}" already exists`);
  await mkdir(path);
}

export async function renamePath(oldPath: string, newPath: string): Promise<void> {
  await renameRaw(oldPath, newPath);
}

export async function deletePath(path: string, isFolder: boolean): Promise<void> {
  await remove(path, { recursive: isFolder });
}

/** Copies a file alongside itself as "name copy.ext", "name copy 2.ext", ... and returns the new path. */
export async function duplicateFile(path: string): Promise<string> {
  const content = await readTextFileRaw(path);
  const dir = await dirname(path);
  const name = await basename(path);
  const dotIndex = name.lastIndexOf(".");
  const stem = dotIndex > 0 ? name.slice(0, dotIndex) : name;
  const ext = dotIndex > 0 ? name.slice(dotIndex) : "";

  let candidateName = `${stem} copy${ext}`;
  let newPath = await join(dir, candidateName);
  let n = 2;
  while (await exists(newPath)) {
    candidateName = `${stem} copy ${n}${ext}`;
    newPath = await join(dir, candidateName);
    n++;
  }
  await writeTextFileRaw(newPath, content);
  return newPath;
}
