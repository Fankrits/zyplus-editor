export interface ExtensionManifest {
  id: string;
  name: string;
  description: string;
  version: string;
  author: string;
  sizeBytesEstimate: number;
  languages: string[];
  /** File name endings (".json") the extension opens in place of the built-in editors. */
  fileExtensions?: string[];
  /** The views that editor offers. The tab bar shows them as a switch, where Rich / Plain are for markdown. */
  fileModes?: { id: string; label: string }[];
  /** Primary download URL from GitHub (releases or raw assets) */
  downloadUrl: string;
  /** SHA-256 (hex) the download must match before it is saved or run. */
  sha256: string;
  /** Optional CSS asset URL if needed */
  cssUrl?: string;
  cssSha256?: string;
}

export interface ExtensionContext {
  isDarkTheme: () => boolean;
}

export interface FileEditorOptions {
  /** The file's text. Read once, at mount; the host remounts the editor when the file changes on disk. */
  initialValue: string;
  /** Reports the whole file's new text after each edit. */
  onChange: (text: string) => void;
  /** The view (one of the manifest's `fileModes`) to open in; the editor picks its own when absent. */
  mode?: string;
  /** Reports the view being shown: once at mount, then whenever it changes. */
  onModeChange: (mode: string) => void;
}

export interface FileEditorHandle {
  destroy: () => void;
  /** Switches view. The host calls it when the user picks one in the tab bar. */
  setMode?: (mode: string) => void;
}

export interface ExtensionRuntime {
  id: string;
  /** Draws an editor for a file the extension claims (see `fileExtensions`) into `host`. */
  mountFileEditor?: (host: HTMLElement, options: FileEditorOptions) => FileEditorHandle;
  renderCodeBlockPreview?: (
    language: string,
    content: string,
    applyPreview: (value: null | string | HTMLElement) => void,
  ) => void | null | string | HTMLElement;
  activate?: (context: ExtensionContext) => void | Promise<void>;
  deactivate?: () => void | Promise<void>;
}

export type ExtensionStatus = "uninstalled" | "downloading" | "installed" | "error";

export interface ExtensionState {
  manifest: ExtensionManifest;
  status: ExtensionStatus;
  errorMessage?: string;
  downloadProgress?: number; // 0 to 100
}
