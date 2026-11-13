import { emit } from "./commands";

/** Tells the toast host a toast is coming, so it loads the toast region. */
export const TOASTS_NEEDED_EVENT = "zyplus:toasts-needed";

type ToastOptions = NonNullable<Parameters<typeof import("@heroui/react").toast.info>[1]>;

/**
 * Shows a toast. HeroUI's toast code (and the react-aria toast region behind it) loads the first time
 * one is shown: most sessions never see one, so it is not part of the page until then. The toast queue
 * is global, so one added just before the region mounts is still shown.
 */
export async function showToast(kind: "info" | "danger", title: string, options?: ToastOptions): Promise<void> {
  emit(TOASTS_NEEDED_EVENT);
  const { toast } = await import("@heroui/react");
  toast[kind](title, options);
}
