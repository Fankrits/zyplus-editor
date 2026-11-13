import { showToast } from "../lib/toasts";
import { extensionManager } from "./extensionManager";

/**
 * Called right after a new tab opens (from disk or freshly created). If the
 * file's extension has a dedicated editor that isn't enabled yet, nudges the
 * user toward it — installing from here swaps the already-open tab over live,
 * since `useFileEditorId` re-checks on every extension state change.
 */
export function suggestExtensionFor(path: string): void {
  const manifest = extensionManager.getAvailableButDisabledExtension(path);
  if (!manifest) return;

  void showToast("info", `${manifest.name} available`, {
    description: `Zyplus has a dedicated editor for this file type. Enable "${manifest.name}" to use it here, or later in Settings → Extensions.`,
    actionProps: {
      children: "Enable",
      onPress: () => {
        extensionManager.downloadAndInstall(manifest.id).catch((err) => {
          void showToast("danger", `Could not install ${manifest.name}`, {
            description: err instanceof Error ? err.message : String(err),
          });
        });
      },
    },
  });
}
