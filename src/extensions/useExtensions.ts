import { useEffect, useState } from "react";
import { extensionManager } from "./extensionManager";
import type { ExtensionState } from "./types";

export function useExtensions(): ExtensionState[] {
  const [states, setStates] = useState(() => extensionManager.getStates());
  // Re-sync on mount too: a change may land between first render and subscribe.
  useEffect(() => {
    setStates(extensionManager.getStates());
    return extensionManager.subscribe(() => setStates(extensionManager.getStates()));
  }, []);
  return states;
}

/** The extension editing this file, or null. Re-renders when an extension is installed or removed. */
export function useFileEditorId(path: string | undefined): string | null {
  useExtensions();
  return path ? extensionManager.getFileEditorId(path) : null;
}
