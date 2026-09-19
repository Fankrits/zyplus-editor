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
import { join as tauriJoin, dirname as tauriDirname, homeDir } from "@tauri-apps/api/path";
import { isTauri, invoke } from "@tauri-apps/api/core";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { isMac, isWindows } from "./platform";
import { onOtherTabWrite, openWebStore, persistWeb, type WebOp } from "./webStore";
import type { TreeNode } from "../state/workspaceReducer";

const MARKDOWN_EXTENSIONS = [".md", ".markdown", ".txt"];

export function isMarkdownFile(name: string): boolean {
  const lower = name.toLowerCase();
  return MARKDOWN_EXTENSIONS.some((ext) => lower.endsWith(ext));
}

/**
 * The web build's filesystem, in memory: files by path, plus folders, which can
 * be empty. Seeded with a demo workspace for the tests; `initWebFs` replaces it
 * with what the browser has stored (see `lib/webStore.ts`).
 */
const webFiles = new Map<string, string>([
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
const webDirs = new Set<string>(["/demo-workspace"]);

/** Where the web build keeps notes. There is only the one folder. */
export const WEB_ROOT = "/Zyplus";

/** Loads the web build's stored files. Call once before rendering; a no-op on desktop. */
export async function initWebFs(): Promise<void> {
  if (isTauri()) return;
  webFiles.clear();
  webDirs.clear();
  try {
    for (const [path, value] of await openWebStore()) {
      if (value === null) webDirs.add(path);
      else webFiles.set(path, value);
    }
  } catch (err) {
    // Private windows in some browsers refuse IndexedDB. The app still works,
    // it just forgets everything on reload.
    console.error("Browser storage is unavailable; notes will not be kept:", err);
  }

  // Another tab of the same app writing to the store it shares with this one.
  onOtherTabWrite((ops) => {
    applyToMemory(ops);
    onStoreChanged?.(ops.map(([path]) => path));
  });
}

/**
 * Notified when the notes changed without this tab writing them — another tab of
 * the web build did. Registered by the workspace store, which reloads the tree
 * and any open tab the same way it does after a sync pull.
 */
let onStoreChanged: ((paths: string[]) => void) | null = null;

export function setStoreChangedListener(fn: ((paths: string[]) => void) | null): void {
  onStoreChanged = fn;
}

function applyToMemory(ops: WebOp[]): void {
  for (const [path, value] of ops) {
    if (value === undefined) {
      webFiles.delete(path);
      webDirs.delete(path);
    } else if (value === null) {
      webDirs.add(path);
    } else {
      webFiles.set(path, value);
    }
  }
}

/** Writes to the browser store first, then to memory, so memory never runs ahead of it. */
async function applyWeb(ops: WebOp[]): Promise<void> {
  await persistWeb(ops);
  applyToMemory(ops);
}

/** `path` and everything under it, as [path, content-or-null-for-a-folder]. */
function webEntriesUnder(path: string): [string, string | null][] {
  const inside = (p: string) => p === path || p.startsWith(`${path}/`);
  return [
    ...[...webFiles].filter(([p]) => inside(p)),
    ...[...webDirs].filter(inside).map((p): [string, null] => [p, null]),
  ];
}

/** Immediate children of a folder. A file deep in a folder makes its parents exist too. */
function webReadDir(dir: string): { name: string; isDirectory: boolean }[] {
  const prefix = `${dir.replace(/\/+$/, "")}/`;
  const children = new Map<string, boolean>();
  for (const p of [...webFiles.keys(), ...webDirs]) {
    if (!p.startsWith(prefix)) continue;
    const rest = p.slice(prefix.length);
    const slash = rest.indexOf("/");
    if (slash === -1) children.set(rest, webDirs.has(p));
    else children.set(rest.slice(0, slash), true);
  }
  return [...children].map(([name, isDirectory]) => ({ name, isDirectory }));
}

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

/**
 * A picker that fails is reported and then treated as cancelled. Every caller
 * already handles "nothing picked"; none handled a rejection, which surfaced
 * as nothing at all.
 */
async function reportingPicker(title: string, fn: () => Promise<string | null>): Promise<string | null> {
  let picked: string | null = null;
  await tryFs(title, "", async () => {
    picked = await fn();
  });
  return picked;
}

/** On the web there are no folders to pick, so this is always the notes folder. */
export function openFolderDialog(): Promise<string | null> {
  return reportingPicker("Could not choose a folder", async () => {
    if (!isTauri()) {
      await ensureFolder(WEB_ROOT);
      return WEB_ROOT;
    }
    const result = await openDialog({ directory: true });
    return typeof result === "string" ? result : null;
  });
}

export function openFileDialog(): Promise<string | null> {
  return reportingPicker("Could not open a file", pickFile);
}

async function pickFile(): Promise<string | null> {
  if (!isTauri()) return importFromDevice();
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

/**
 * A browser cannot open a file in place, so the picked file is copied into the
 * notes folder, next to anything already named the same rather than over it.
 */
async function importFromDevice(): Promise<string | null> {
  const file = await new Promise<File | null>((resolve) => {
    const input = Object.assign(document.createElement("input"), {
      type: "file",
      accept: MARKDOWN_EXTENSIONS.join(","),
    });
    input.onchange = () => resolve(input.files?.[0] ?? null);
    input.oncancel = () => resolve(null);
    input.click();
  });
  if (!file) return null;
  await ensureFolder(WEB_ROOT);
  const path = await freePath(WEB_ROOT, file.name);
  await writeTextFile(path, await file.text());
  return path;
}

/** `dir/name`, or `dir/name<label> 2`, `… 3` and so on until nothing is there. */
async function freePath(dir: string, name: string, label = ""): Promise<string> {
  const dot = name.lastIndexOf(".");
  const stem = dot > 0 ? name.slice(0, dot) : name;
  const ext = dot > 0 ? name.slice(dot) : "";
  let path = await joinPath(dir, `${stem}${label}${ext}`);
  for (let n = 2; await pathExists(path); n++) {
    path = await joinPath(dir, `${stem}${label} ${n}${ext}`);
  }
  return path;
}

/** Reads a directory tree top to bottom, skipping dotfiles/dot-directories. */
export async function readDirRecursive(dirPath: string): Promise<TreeNode[]> {
  const entries = isTauri() ? await readDir(dirPath) : webReadDir(dirPath);
  // Subfolders are read together rather than one after another: a tree three
  // levels deep used to cost the sum of every folder in it, in series, and this
  // runs at startup and again after every sync pull that touched anything.
  // ponytail: no concurrency cap — a notebook's worth of folders is fine; a disk
  // with thousands would want one.
  const nodes = await Promise.all(
    entries
      .filter((entry) => !entry.name.startsWith("."))
      .map(async (entry): Promise<TreeNode> => {
        const path = displayJoin(dirPath, entry.name);
        return entry.isDirectory
          ? { id: path, name: entry.name, isFolder: true, children: await readDirRecursive(path) }
          : { id: path, name: entry.name, isFolder: false };
      }),
  );
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
 * Joins a parent and one child name using the separator `parent` already uses —
 * Windows hands back `C:\\Users\\me\\Documents`, and pasting a "/" onto that
 * renders a path the user has never seen.
 *
 * This is the one to reach for when walking a directory. `joinPath` is Tauri's
 * `join`, which is native IPC: a round trip per call, and reading a tree of a
 * few hundred notes made one per file. A child of a directory the OS just handed
 * us needs no normalising, so the string concatenation is both correct and free.
 */
export function displayJoin(parent: string, name: string): string {
  const sep = parent.includes("\\") && !parent.includes("/") ? "\\" : "/";
  return parent.endsWith(sep) ? `${parent}${name}` : `${parent}${sep}${name}`;
}

/** Suggested parent for the default folder: home, since macOS prompts for every access to Documents. */
export async function defaultFolderParent(): Promise<string> {
  if (!isTauri()) return "/";
  return homeDir();
}

/** Creates (or reuses) `<parent>/Zyplus` and returns its path. */
export async function createDefaultFolder(parent: string): Promise<string> {
  const path = await joinPath(parent, DEFAULT_FOLDER_NAME);
  await ensureFolder(path);
  return path;
}

/**
 * Notified after every change this module makes to disk, so the sync layer can
 * mirror it. Registered by `lib/sync.ts`; a no-op until then, and left alone
 * entirely when the user is signed out.
 *
 * It lives here rather than at the call sites — `saveDocument`, the context
 * menu, autosave, the tab bar — so that a save path added later is synced
 * without anyone remembering to wire it up.
 */
let onLocalChange: ((path: string) => void) | null = null;

export function setLocalChangeListener(fn: ((path: string) => void) | null): void {
  onLocalChange = fn;
}

/** Whether a path exists. Sync needs this to tell a delete from a write. */
export async function pathExists(path: string): Promise<boolean> {
  if (!isTauri()) return webEntriesUnder(path).length > 0;
  return exists(path);
}

/** Creates a folder and any missing parents. Unlike `createFolder`, existing is fine. */
export async function ensureFolder(path: string): Promise<void> {
  if (isTauri()) {
    await mkdir(path, { recursive: true });
    return;
  }
  const missing: WebOp[] = [];
  for (let p = path; p !== "/" && !webDirs.has(p); p = mockDirname(p)) missing.push([p, null]);
  await applyWeb(missing);
}

export async function readTextFile(path: string): Promise<string> {
  if (!isTauri()) {
    const content = webFiles.get(path);
    if (content === undefined) throw new Error(`File not found: ${path}`);
    return content;
  }
  return readTextFileRaw(path);
}

export async function writeTextFile(path: string, content: string): Promise<void> {
  if (!isTauri()) {
    await applyWeb([[path, content]]);
  } else {
    await writeTextFileRaw(path, content);
  }
  onLocalChange?.(path);
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
    try {
      if (isTauri()) await message(explainFsError(err, path), { title, kind: "error" });
      else window.alert(`${title}\n\n${explainFsError(err, path)}`);
    } catch {
      // Reporting the failure failing is not worth a second failure path.
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
  if (await pathExists(path)) throw new Error(`"${path}" already exists`);
  await writeTextFile(path, "");
}

export async function createFolder(path: string): Promise<void> {
  assertNameIsUsable(path);
  if (await pathExists(path)) throw new Error(`"${path}" already exists`);
  // Recursive, to match createDefaultFolder — the exists guard above already
  // covers the collision that non-recursive mkdir was catching by accident.
  await ensureFolder(path);
}

/** `path` is `prefix` itself or something inside it. */
export function isUnder(path: string, prefix: string): boolean {
  return path === prefix || path.startsWith(prefix + "/") || path.startsWith(prefix + "\\");
}

export async function renamePath(oldPath: string, newPath: string): Promise<void> {
  // Only a name the user just typed gets validated. Dragging a file that
  // already carries an awkward name — legal on macOS, not on Windows — into
  // another folder must keep working; refusing that would strand the file.
  if (basenameOf(newPath) !== basenameOf(oldPath)) assertNameIsUsable(newPath);
  if (isUnder(newPath, oldPath) && newPath !== oldPath) {
    throw new Error(`Cannot move "${basenameOf(oldPath)}" into itself`);
  }
  // rename(2) replaces an existing file, and the web store would too. A
  // case-only rename finds itself on a case-insensitive disk, which is fine.
  if (newPath.toLowerCase() !== oldPath.toLowerCase() && (await pathExists(newPath))) {
    throw new Error(`"${basenameOf(newPath)}" already exists there`);
  }
  if (!isTauri()) {
    const moved = webEntriesUnder(oldPath);
    await applyWeb([
      ...moved.map(([p]): WebOp => [p, undefined]),
      ...moved.map(([p, value]): WebOp => [newPath + p.slice(oldPath.length), value]),
    ]);
  } else {
    await renameRaw(oldPath, newPath);
  }
  // Both ends: the old path becomes a delete upstream, the new one an upload.
  onLocalChange?.(oldPath);
  onLocalChange?.(newPath);
}

export async function deletePath(path: string, isFolder: boolean): Promise<void> {
  if (!isTauri()) {
    await applyWeb(webEntriesUnder(path).map(([p]): WebOp => [p, undefined]));
  } else {
    await remove(path, { recursive: isFolder });
  }
  onLocalChange?.(path);
}

/** Copies a file alongside itself as "name copy.ext", "name copy 2.ext", ... and returns the new path. */
export async function duplicateFile(path: string): Promise<string> {
  const content = await readTextFile(path);
  const newPath = await freePath(await dirnameOf(path), basenameOf(path), " copy");
  await writeTextFile(newPath, content);
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
