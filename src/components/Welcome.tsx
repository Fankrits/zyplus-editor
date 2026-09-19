import { useCallback, useEffect, useState } from "react";
import { Button } from "@heroui/react";
import { HugeiconsIcon } from "@hugeicons/react";
import { FolderAddIcon } from "@hugeicons/core-free-icons";
import * as fs from "../lib/fs";
import { Logo } from "./Logo";

export interface WelcomeProps {
  onReady: (folder: string) => void;
  onSkip?: () => void;
  onOpenFile?: () => void;
  onOpenFolder?: () => void;
}

/**
 * First-run screen: the app can initialize a default notes folder, or let the user
 * jump straight into opening an existing file or project.
 */
export function Welcome({ onReady, onSkip, onOpenFile, onOpenFolder }: WelcomeProps) {
  const [folder, setFolder] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    fs.defaultFolderPath().then(setFolder).catch(() => setFolder(null));
  }, []);

  const create = useCallback(async () => {
    if (!folder) return;
    setBusy(true);
    setError(null);
    try {
      onReady(await fs.createDefaultFolder());
    } catch (err) {
      setError(fs.explainFsError(err, folder));
    } finally {
      setBusy(false);
    }
  }, [folder, onReady]);

  return (
    <div className="flex h-app w-full flex-col items-center justify-center gap-4 bg-background p-6 text-foreground">
      <Logo size={48} className="text-foreground" />
      <h1 className="text-lg font-semibold tracking-tight">Welcome to Zyplus</h1>
      <p className="max-w-sm text-center text-sm text-muted">
        Zyplus can organize your notes in a dedicated folder, or you can open existing files directly.
      </p>
      {folder && (
        <code className="max-w-full truncate rounded-lg bg-surface-secondary px-3 py-1.5 text-xs">
          {folder}
        </code>
      )}
      {error && (
        <div className="max-w-md rounded-lg bg-danger-soft p-3 text-sm text-danger">
          <p className="whitespace-pre-line">{error}</p>
        </div>
      )}
      <div className="flex flex-wrap items-center justify-center gap-2">
        <Button variant="primary" onPress={create} isDisabled={!folder || busy}>
          <HugeiconsIcon icon={FolderAddIcon} size={18} />
          Create folder
        </Button>
        {onOpenFolder && (
          <Button variant="ghost" size="sm" onPress={onOpenFolder} isDisabled={busy}>
            Open project
          </Button>
        )}
        {onOpenFile && (
          <Button variant="ghost" size="sm" onPress={onOpenFile} isDisabled={busy}>
            Open file
          </Button>
        )}
        {onSkip && (
          <Button variant="ghost" size="sm" onPress={onSkip} isDisabled={busy}>
            Skip
          </Button>
        )}
      </div>
    </div>
  );
}
