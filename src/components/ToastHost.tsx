import { lazy, useEffect, useState } from "react";
import { OnDemand } from "./OnDemand";
import { TOASTS_NEEDED_EVENT } from "../lib/toasts";

const ToastRegion = lazy(() => import("@heroui/react").then((m) => ({ default: m.ToastProvider })));

/** Renders HeroUI's toast region once something has asked for a toast (see lib/toasts.ts). */
export function ToastHost() {
  const [needed, setNeeded] = useState(false);
  useEffect(() => {
    const on = () => setNeeded(true);
    window.addEventListener(TOASTS_NEEDED_EVENT, on);
    return () => window.removeEventListener(TOASTS_NEEDED_EVENT, on);
  }, []);
  return (
    <OnDemand when={needed}>
      <ToastRegion />
    </OnDemand>
  );
}
