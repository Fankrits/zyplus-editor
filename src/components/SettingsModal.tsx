import { useState } from "react";
import { Button, Modal } from "@heroui/react";
import { HugeiconsIcon } from "@hugeicons/react";
import { Settings01Icon, FolderOpenIcon } from "@hugeicons/core-free-icons";
import { useWorkspace } from "../state/workspaceStore";
import * as fs from "../lib/fs";
import { getTheme, setTheme, type Theme } from "../lib/theme";

const THEMES: Theme[] = ["system", "light", "dark"];

const MOD = typeof navigator !== "undefined" && /Mac|iP(hone|ad)/.test(navigator.userAgent) ? "⌘" : "Ctrl";

const SHORTCUTS: [string, string][] = [
  [`${MOD} S`, "Save the current file"],
  [`${MOD} N`, "New file"],
  [`${MOD} B`, "Toggle sidebar"],
];

export function SettingsModal({ isOpen, onClose }: { isOpen: boolean; onClose: () => void }) {
  const { state, dispatch, defaultFolder, setDefaultFolder } = useWorkspace();
  const [theme, setThemeState] = useState<Theme>(getTheme);

  const pickTheme = (next: Theme) => {
    setTheme(next);
    setThemeState(next);
  };

  const pickDefaultFolder = async () => {
    const picked = await fs.openFolderDialog();
    if (!picked) return;
    setDefaultFolder(picked);
    if (!state.roots.includes(picked)) {
      dispatch({ type: "ADD_ROOT", rootPath: picked, node: await fs.readProjectNode(picked) });
    }
  };

  return (
    <Modal>
      <Modal.Backdrop isOpen={isOpen} onOpenChange={(open) => !open && onClose()}>
        <Modal.Container size="sm">
          <Modal.Dialog>
            <Modal.CloseTrigger />
            <Modal.Header>
              <Modal.Icon className="size-8 bg-accent-soft text-accent">
                <HugeiconsIcon icon={Settings01Icon} size={16} />
              </Modal.Icon>
              <Modal.Heading>Settings</Modal.Heading>
            </Modal.Header>
            <Modal.Body className="flex flex-col gap-5">
              <section>
                <h3 className="mb-2 text-xs font-medium uppercase tracking-wide text-muted">
                  Theme
                </h3>
                <div className="flex gap-2">
                  {THEMES.map((t) => (
                    <Button
                      key={t}
                      size="sm"
                      className="flex-1 capitalize"
                      variant={theme === t ? "primary" : "secondary"}
                      onPress={() => pickTheme(t)}
                    >
                      {t}
                    </Button>
                  ))}
                </div>
              </section>

              <section>
                <h3 className="mb-2 text-xs font-medium uppercase tracking-wide text-muted">
                  Default folder
                </h3>
                <div className="flex items-center gap-2">
                  <span className="min-w-0 flex-1 truncate text-sm" title={defaultFolder ?? undefined}>
                    {defaultFolder ?? "Not set"}
                  </span>
                  <Button size="sm" variant="secondary" onPress={pickDefaultFolder}>
                    <HugeiconsIcon icon={FolderOpenIcon} size={16} />
                    Change
                  </Button>
                </div>
                <p className="mt-1 text-xs text-muted">New files land here by default.</p>
              </section>

              <section>
                <h3 className="mb-2 text-xs font-medium uppercase tracking-wide text-muted">
                  Shortcuts
                </h3>
                <ul className="flex flex-col gap-1.5">
                  {SHORTCUTS.map(([keys, label]) => (
                    <li key={keys} className="flex items-center justify-between text-sm">
                      <span>{label}</span>
                      <kbd className="rounded border border-border px-1.5 py-0.5 text-xs">
                        {keys}
                      </kbd>
                    </li>
                  ))}
                </ul>
              </section>
            </Modal.Body>
            <Modal.Footer>
              <Button size="sm" variant="ghost" onPress={onClose}>
                Close
              </Button>
            </Modal.Footer>
          </Modal.Dialog>
        </Modal.Container>
      </Modal.Backdrop>
    </Modal>
  );
}
