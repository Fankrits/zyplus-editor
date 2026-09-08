import { open as openDialog, save as saveDialog, message } from "@tauri-apps/plugin-dialog";
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
import { isTauri, invoke } from "@tauri-apps/api/core";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { isMac, isWindows } from "./platform";
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

/**
 * `join`/`dirname`, guarded. Tauri's path API is native IPC and throws outside
 * the desktop app, so call sites used to break the browser build and the tests
 * every time one reached for it directly.
 */
export async function joinPath(...parts: string[]): Promise<string> {
  return isTauri() ? tauriJoin(...parts) : mockJoin(...parts);
}

export async function dirnameOf(path: string): Promise<string> {
  return isTauri() ? tauriDirname(path) : mockDirname(path);
}

/** Last segment of a path, either separator. The one copy in the app. */
export function basenameOf(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

// Windows' rules, enforced on every platform. A note named "a:b.md" created on
// a Mac cannot be copied to a Windows machine at all, so accepting it here only
// moves the failure somewhere the user will not see it.
const RESERVED_ON_WINDOWS = /^(con|prn|aux|nul|com[1-9]|lpt[1-9])(\..*)?$/i;

/** Why `name` cannot be a filename, or null if it can. */
export function invalidNameReason(name: string): string | null {
  if (!name) return "Enter a name.";
  if (name === "." || name === "..") return "Choose a different name.";
  if (/[\\/]/.test(name)) return "A name cannot contain \\ or /.";
  if (/[<>:"|?*]/.test(name)) return 'A name cannot contain < > : " | ? *';
  if (/[\x00-\x1f]/.test(name)) return "A name cannot contain control characters.";
  if (name.endsWith(".")) return "A name cannot end with a period.";
  if (name.endsWith(" ")) return "A name cannot end with a space.";
  if (RESERVED_ON_WINDOWS.test(name)) return `"${name}" is a name Windows reserves.`;
  if (name.length > 255) return "That name is too long.";
  return null;
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
  return { id: dirPath, name: basenameOf(dirPath), isFolder: true, children };
}

export const DEFAULT_FOLDER_NAME = "Zyplus";

/**
 * Joins for display only, using the separator `parent` already uses — Windows
 * hands back `C:\\Users\\me\\Documents`, and pasting a "/" onto that renders a
 * path the user has never seen. Real paths still go through Tauri's `join`.
 */
export function displayJoin(parent: string, name: string): string {
  const sep = parent.includes("\\") && !parent.includes("/") ? "\\" : "/";
  return parent.endsWith(sep) ? `${parent}${name}` : `${parent}${sep}${name}`;
}

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
      // The dev server answers *any* unknown path with index.html and a 200, so
      // `res.ok` alone would "open" a document whose content is the app's own
      // markup. Only a non-HTML body is a real file here.
      const type = res.headers.get("content-type") ?? "";
      if (res.ok && !type.includes("text/html")) return await res.text();
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

/** Whether the OS refused us, rather than us having asked for something silly. */
function isPermissionError(detail: string): boolean {
  return /os error 5\b|EACCES|EPERM|permission denied|access is denied/i.test(detail);
}

/**
 * Turns a raw OS error into something the user can act on.
 *
 * A denied write is almost never a broken path — it is the platform's own file
 * protection, and "Access is denied. (os error 5)" tells the user nothing about
 * which switch to flip. Anything we do not recognize passes through untouched.
 */
export function explainFsError(err: unknown, path: string): string {
  const detail = err instanceof Error ? err.message : String(err);
  if (!isPermissionError(detail)) return detail;
  if (isWindows) {
    return (
      `Windows blocked the change to "${path}".\n\n` +
      "This is usually Controlled folder access, which stops apps it does not " +
      "recognize from writing to Documents and Desktop.\n\n" +
      "Either choose a different location, or allow Zyplus: Windows Security → " +
      "Virus & threat protection → Ransomware protection → Allow an app through " +
      "Controlled folder access."
    );
  }
  if (isMac) {
    return (
      `macOS blocked the change to "${path}".\n\n` +
      "Zyplus does not have permission for this folder. Either choose a different " +
      "location, or grant access in System Settings → Privacy & Security → Files " +
      "and Folders."
    );
  }
  return `"${path}" is not writable.\n\nCheck the folder's permissions, or choose a different location.`;
}

/**
 * Runs a filesystem mutation, reporting a failure instead of dropping it.
 * Returns whether it landed, so callers do not refresh the tree, open a tab or
 * mark a document clean over a change that never reached disk.
 *
 * Every mutating call site goes through here. An uncaught throw inside a React
 * event handler is completely invisible, which is what made a blocked write
 * look to the user like the button simply did nothing.
 *
 * Autosave deliberately does not go through here: a background write that keeps
 * failing should stay a console warning, not a dialog on a timer.
 */
export async function tryFs(
  title: string,
  path: string,
  fn: () => Promise<unknown>,
): Promise<boolean> {
  try {
    await fn();
    return true;
  } catch (err) {
    console.error(`${title} — "${path}":`, err);
    if (isTauri()) {
      try {
        await message(explainFsError(err, path), { title, kind: "error" });
      } catch {
        // Reporting the failure failing is not worth a second failure path.
      }
    }
    return false;
  }
}

/** Writes a document the user explicitly asked to save. */
export async function saveDocument(path: string, content: string): Promise<boolean> {
  return tryFs("Save failed", path, () => writeTextFile(path, content));
}

/**
 * Rejects a name the filesystem cannot hold, before we ask it to.
 *
 * This lives down here rather than in the dialogs so that every path into the
 * filesystem is covered — the create modal, inline rename, and anything added
 * later — and so the failure is one readable sentence instead of an OS errno.
 */
function assertNameIsUsable(path: string): void {
  const reason = invalidNameReason(basenameOf(path));
  if (reason) throw new Error(reason);
}

export async function createFile(path: string): Promise<void> {
  assertNameIsUsable(path);
  if (!isTauri()) {
    mockFsStore.set(path, "");
    return;
  }
  if (await exists(path)) throw new Error(`"${path}" already exists`);
  await writeTextFileRaw(path, "");
}

export async function createFolder(path: string): Promise<void> {
  assertNameIsUsable(path);
  if (!isTauri()) return;
  if (await exists(path)) throw new Error(`"${path}" already exists`);
  // Recursive, to match createDefaultFolder — the exists() guard above already
  // covers the collision that non-recursive mkdir was catching by accident.
  await mkdir(path, { recursive: true });
}

export async function renamePath(oldPath: string, newPath: string): Promise<void> {
  // Only a name the user just typed gets validated. Dragging a file that
  // already carries an awkward name — legal on macOS, not on Windows — into
  // another folder must keep working; refusing that would strand the file.
  if (basenameOf(newPath) !== basenameOf(oldPath)) assertNameIsUsable(newPath);
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
    const name = basenameOf(path);
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

/**
 * Shows `path` in the OS file manager. A no-op in the browser build, and a
 * logged warning rather than an unhandled rejection when the OS declines —
 * every caller is a fire-and-forget menu item.
 */
export async function revealPath(path: string): Promise<void> {
  if (!isTauri()) return;
  try {
    await revealItemInDir(path);
  } catch (err) {
    console.error(`Could not reveal "${path}":`, err);
  }
}

/** Files the OS handed us (double-click / "Open With"). Drains the native queue. */
export async function takePendingFiles(): Promise<string[]> {
  if (!isTauri()) return [];
  return invoke<string[]>("take_pending_files");
}

/** Registers Zyplus as the system handler for Markdown. macOS only. */
export async function setDefaultMarkdownApp(): Promise<void> {
  if (!isTauri()) throw new Error("Only available in the desktop app.");
  await invoke("set_default_markdown_app");
}

/** Whether Markdown already opens in Zyplus. */
export async function isDefaultMarkdownApp(): Promise<boolean> {
  if (!isTauri()) return false;
  return invoke<boolean>("is_default_markdown_app");
}
