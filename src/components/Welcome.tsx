import { useCallback, useEffect, useState } from "react";
import { Button } from "@heroui/react";
import { HugeiconsIcon } from "@hugeicons/react";
import { FolderAddIcon, FolderOpenIcon } from "@hugeicons/core-free-icons";
import * as fs from "../lib/fs";
import { Logo } from "./Logo";

/**
 * First-run screen: the app needs one folder of its own before it can store anything.
 * Creates `<Documents>/Zyplus` by default; the location can be changed first.
 */
export function Welcome({ onReady }: { onReady: (folder: string) => void }) {
  const [parent, setParent] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    fs.defaultFolderParent().then(setParent).catch(() => setParent(null));
  }, []);

  const chooseParent = useCallback(async () => {
    const picked = await fs.openFolderDialog();
    if (picked) setParent(picked);
  }, []);

  const create = useCallback(async () => {
    if (!parent) return;
    setBusy(true);
    setError(null);
    try {
      onReady(await fs.createDefaultFolder(parent));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }, [parent, onReady]);

  return (
    <div className="flex h-screen w-screen flex-col items-center justify-center gap-4 bg-white p-6 text-black dark:bg-neutral-900 dark:text-white">
      <Logo size={48} className="text-black dark:text-white" />
      <h1 className="text-lg font-semibold tracking-tight">Welcome to Zyplus</h1>
      <p className="max-w-sm text-center text-sm text-neutral-500">
        Zyplus keeps your notes in a folder on your computer. Pick where it should live — you can
        add more project folders later.
      </p>
      <code className="max-w-full truncate rounded-lg bg-black/5 px-3 py-1.5 text-xs dark:bg-white/10">
        {parent ? `${parent}/${fs.DEFAULT_FOLDER_NAME}` : "Loading…"}
      </code>
      {error && <p className="text-sm text-danger">{error}</p>}
      <div className="flex gap-2">
        <Button variant="ghost" size="sm" onPress={chooseParent} isDisabled={busy}>
          <HugeiconsIcon icon={FolderOpenIcon} size={16} />
          Change location
        </Button>
        <Button variant="primary" onPress={create} isDisabled={!parent || busy}>
          <HugeiconsIcon icon={FolderAddIcon} size={18} />
          Create folder
        </Button>
      </div>
    </div>
  );
}
