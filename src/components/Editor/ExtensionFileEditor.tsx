import { useEffect, useRef, useState } from "react";
import { extensionManager } from "../../extensions/extensionManager";
import type { FileEditorHandle } from "../../extensions/types";
import { isMac } from "../../lib/platform";
import { matchShortcut } from "../../lib/shortcuts";

interface ExtensionFileEditorProps {
  extensionId: string;
  initialValue: string;
  onChange: (text: string) => void;
  /** The view to show (one of the extension's `fileModes`), driven from the tab bar. */
  mode?: string;
  onModeChange: (mode: string) => void;
}

/** Hosts an editor an extension draws itself (see `mountFileEditor`), in place of the built-in ones. */
export function ExtensionFileEditor({
  extensionId,
  initialValue,
  onChange,
  mode,
  onModeChange,
}: ExtensionFileEditorProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const [failed, setFailed] = useState(false);
  // The extension is mounted once; edits reach whichever callback is current.
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;
  const onModeChangeRef = useRef(onModeChange);
  onModeChangeRef.current = onModeChange;
  const handleRef = useRef<FileEditorHandle | undefined>(undefined);

  useEffect(() => {
    let handle: FileEditorHandle | undefined;
    let cancelled = false;
    extensionManager.loadRuntime(extensionId).then(
      (runtime) => {
        if (cancelled || !hostRef.current) return;
        handle = runtime.mountFileEditor?.(hostRef.current, {
          // Read at mount only, like the built-in editors; EditorPane remounts on reload.
          initialValue,
          onChange: (text) => onChangeRef.current(text),
          // Also read at mount only: after that the tab bar drives it, through `setMode` below.
          mode,
          onModeChange: (next) => onModeChangeRef.current(next),
        });
        handleRef.current = handle;
        if (!handle) setFailed(true);
      },
      () => setFailed(true),
    );
    return () => {
      cancelled = true;
      handle?.destroy();
      handleRef.current = undefined;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [extensionId]);

  // The user picked a view in the tab bar. Before the editor has mounted this finds no handle,
  // which is right: it opens in `mode` anyway.
  useEffect(() => {
    if (mode) handleRef.current?.setMode?.(mode);
  }, [mode]);

  // An extension's editor may stop keydown from bubbling (the JSON editor does, for
  // every key), which would leave ⌘S, ⌘W and the rest dead while it has focus. The
  // capture phase runs before it, so the app's own window handler still gets them,
  // sent from `document` the way the browser would have bubbled them. Find stays
  // with the editor, which has its own.
  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const forward = (e: KeyboardEvent) => {
      if (!(isMac ? e.metaKey : e.ctrlKey)) return;
      const command = matchShortcut(e);
      if (command === "find" || command === "replace") return;
      const copy = new KeyboardEvent("keydown", {
        key: e.key,
        code: e.code,
        metaKey: e.metaKey,
        ctrlKey: e.ctrlKey,
        shiftKey: e.shiftKey,
        altKey: e.altKey,
        bubbles: true,
        cancelable: true,
      });
      // The app cancels what it handles; cancel the original too, or a browser saves the page.
      if (!document.dispatchEvent(copy)) e.preventDefault();
    };
    host.addEventListener("keydown", forward, true);
    return () => host.removeEventListener("keydown", forward, true);
  }, [failed]);

  if (failed) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-muted">
        This extension could not be loaded. Reinstall it from Settings → Extensions.
      </div>
    );
  }
  return <div ref={hostRef} className="h-full" />;
}
