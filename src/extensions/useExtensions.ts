import { useEffect, useState } from "react";
import { extensionManager } from "./extensionManager";
import type { ExtensionState } from "./types";

export function useExtensions(): {
  states: ExtensionState[];
  downloadAndInstall: (id: string) => Promise<void>;
  uninstallAndRemove: (id: string) => Promise<void>;
} {
  const [states, setStates] = useState<ExtensionState[]>(() =>
    extensionManager.getStates(),
  );

  useEffect(() => {
    // Initial sync
    setStates(extensionManager.getStates());
    // Subscribe to changes
    return extensionManager.subscribe(() => {
      setStates(extensionManager.getStates());
    });
  }, []);

  return {
    states,
    downloadAndInstall: (id: string) => extensionManager.downloadAndInstall(id),
    uninstallAndRemove: (id: string) => extensionManager.uninstallAndRemove(id),
  };
}

/** The extension editing this file, or null. Re-renders when an extension is installed or removed. */
export function useFileEditorId(path: string | undefined): string | null {
  useExtensions();
  return path ? extensionManager.getFileEditorId(path) : null;
}
