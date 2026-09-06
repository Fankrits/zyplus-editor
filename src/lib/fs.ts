import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import {
  readDir,
  readTextFile as readTextFileRaw,
  writeTextFile as writeTextFileRaw,
  mkdir,
  remove,
  rename as renameRaw,
  exists,
} from "@tauri-apps/plugin-fs";
import {
  join as tauriJoin,
  dirname as tauriDirname,
  basename as tauriBasename,
  documentDir,
} from "@tauri-apps/api/path";
import { isTauri } from "@tauri-apps/api/core";
import type { TreeNode } from "../state/workspaceReducer";

const MARKDOWN_EXTENSIONS = [".md", ".markdown", ".txt"];

export function isMarkdownFile(name: string): boolean {
  const lower = name.toLowerCase();
  return MARKDOWN_EXTENSIONS.some((ext) => lower.endsWith(ext));
}

// In-memory filesystem for browser development / headless tests without native Tauri IPC
const mockFsStore = new Map<string, string>([
  [
    "/demo-workspace/README.md",
    "# Tauri + React + Typescript\n\nThis template should help get you started developing with Tauri, React and Typescript in Vite.\n",
  ],
  [
    "/demo-workspace/notes.md",
    "## Project Notes\n\n- Support session restoration\n- Fast and responsive\n",
  ],
  [
    "/demo-workspace/diagram.md",
    "# Diagram & Math Demo\n\n## Flowchart\n\n```mermaid\nflowchart TD\n    A[Start] --> B{Is it working?}\n    B -- Yes --> C[Great!]\n    B -- No --> D[Debug]\n```\n\n## Math Formula\n\n```latex\n\\int_{-\\infty}^\\infty e^{-x^2} dx = \\sqrt{\\pi}\n```\n",
  ],
]);

function mockJoin(...parts: string[]): string {
  return parts.join("/").replace(/\/+/g, "/");
}

function mockDirname(p: string): string {
  const parts = p.split(/[\\/]/);
  parts.pop();
  return parts.join("/") || "/";
}

function mockBasename(p: string): string {
  const parts = p.split(/[\\/]/);
  return parts.pop() || p;
}

export async function openFolderDialog(): Promise<string | null> {
  if (!isTauri()) {
    return "/demo-workspace";
  }
  const result = await openDialog({ directory: true });
  return typeof result === "string" ? result : null;
}

export async function openFileDialog(): Promise<string | null> {
  if (!isTauri()) {
    return "/demo-workspace/README.md";
  }
  const result = await openDialog({
    directory: false,
    multiple: false,
    filters: [
      { name: "Markdown & Text", extensions: ["md", "markdown", "txt"] },
      { name: "All Files", extensions: ["*"] },
    ],
  });
  return typeof result === "string" ? result : null;
}

/** Reads a directory tree top to bottom, skipping dotfiles/dot-directories. */
export async function readDirRecursive(dirPath: string): Promise<TreeNode[]> {
  if (!isTauri()) {
    const nodes: TreeNode[] = [];
    for (const [filePath] of mockFsStore) {
      if (filePath.startsWith(dirPath)) {
        const rel = filePath.slice(dirPath.length).replace(/^[\\/]/, "");
        if (!rel.includes("/") && !rel.includes("\\")) {
          nodes.push({ id: filePath, name: rel, isFolder: false });
        }
      }
    }
    nodes.sort((a, b) => a.name.localeCompare(b.name));
    return nodes;
  }

  const entries = await readDir(dirPath);
  const nodes: TreeNode[] = [];
  for (const entry of entries) {
    if (entry.name.startsWith(".")) continue;
    const path = await tauriJoin(dirPath, entry.name);
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

/** Reads a folder as a project: one collapsible top-level node holding the tree. */
export async function readProjectNode(dirPath: string): Promise<TreeNode> {
  const children = await readDirRecursive(dirPath);
  return { id: dirPath, name: mockBasename(dirPath), isFolder: true, children };
}

export const DEFAULT_FOLDER_NAME = "Zyplus";

/** Suggested parent for the default folder (Documents on desktop). */
export async function defaultFolderParent(): Promise<string> {
  if (!isTauri()) return "/demo";
  return documentDir();
}

/** Creates (or reuses) `<parent>/Zyplus` and returns its path. */
export async function createDefaultFolder(parent: string): Promise<string> {
  const path = isTauri()
    ? await tauriJoin(parent, DEFAULT_FOLDER_NAME)
    : "/demo-workspace";
  if (!isTauri()) return path;
  if (!(await exists(path))) await mkdir(path, { recursive: true });
  return path;
}

export async function readTextFile(path: string): Promise<string> {
  if (!isTauri()) {
    const content = mockFsStore.get(path);
    if (content !== undefined) return content;
    try {
      const res = await fetch(path);
      if (res.ok) return await res.text();
    } catch {}
    throw new Error(`File not found: ${path}`);
  }
  return readTextFileRaw(path);
}

export async function writeTextFile(path: string, content: string): Promise<void> {
  if (!isTauri()) {
    mockFsStore.set(path, content);
    return;
  }
  await writeTextFileRaw(path, content);
}

export async function createFile(path: string): Promise<void> {
  if (!isTauri()) {
    mockFsStore.set(path, "");
    return;
  }
  if (await exists(path)) throw new Error(`"${path}" already exists`);
  await writeTextFileRaw(path, "");
}

export async function createFolder(path: string): Promise<void> {
  if (!isTauri()) return;
  if (await exists(path)) throw new Error(`"${path}" already exists`);
  await mkdir(path);
}

export async function renamePath(oldPath: string, newPath: string): Promise<void> {
  if (!isTauri()) {
    const content = mockFsStore.get(oldPath) ?? "";
    mockFsStore.delete(oldPath);
    mockFsStore.set(newPath, content);
    return;
  }
  await renameRaw(oldPath, newPath);
}

export async function deletePath(path: string, isFolder: boolean): Promise<void> {
  if (!isTauri()) {
    mockFsStore.delete(path);
    return;
  }
  await remove(path, { recursive: isFolder });
}

/** Copies a file alongside itself as "name copy.ext", "name copy 2.ext", ... and returns the new path. */
export async function duplicateFile(path: string): Promise<string> {
  if (!isTauri()) {
    const content = mockFsStore.get(path) ?? "";
    const dir = mockDirname(path);
    const name = mockBasename(path);
    const dotIndex = name.lastIndexOf(".");
    const stem = dotIndex > 0 ? name.slice(0, dotIndex) : name;
    const ext = dotIndex > 0 ? name.slice(dotIndex) : "";
    const newPath = mockJoin(dir, `${stem} copy${ext}`);
    mockFsStore.set(newPath, content);
    return newPath;
  }

  const content = await readTextFileRaw(path);
  const dir = await tauriDirname(path);
  const name = await tauriBasename(path);
  const dotIndex = name.lastIndexOf(".");
  const stem = dotIndex > 0 ? name.slice(0, dotIndex) : name;
  const ext = dotIndex > 0 ? name.slice(dotIndex) : "";

  let candidateName = `${stem} copy${ext}`;
  let newPath = await tauriJoin(dir, candidateName);
  let n = 2;
  while (await exists(newPath)) {
    candidateName = `${stem} copy ${n}${ext}`;
    newPath = await tauriJoin(dir, candidateName);
    n++;
  }
  await writeTextFileRaw(newPath, content);
  return newPath;
}

/** Writes `content` to a path the user picks. Outside Tauri, falls back to a browser download. */
export async function saveFileAs(defaultName: string, content: string): Promise<void> {
  if (!isTauri()) {
    const url = URL.createObjectURL(new Blob([content], { type: "text/markdown" }));
    const link = Object.assign(document.createElement("a"), { href: url, download: defaultName });
    link.click();
    URL.revokeObjectURL(url);
    return;
  }
  const path = await saveDialog({ defaultPath: defaultName });
  if (path) await writeTextFileRaw(path, content);
}
