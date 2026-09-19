export interface ExtensionManifest {
  id: string;
  name: string;
  description: string;
  version: string;
  author: string;
  sizeBytesEstimate: number;
  languages: string[];
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

export interface ExtensionRuntime {
  id: string;
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
