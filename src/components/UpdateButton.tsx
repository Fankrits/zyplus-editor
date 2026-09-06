import { useState } from "react";
import { Button } from "@heroui/react";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

type Status =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "latest" }
  | { kind: "downloading"; version: string }
  | { kind: "ready" }
  | { kind: "error"; message: string };

/** "Check for updates" in About: checks GitHub releases, installs, relaunches. */
export function UpdateButton() {
  const [status, setStatus] = useState<Status>({ kind: "idle" });

  async function run() {
    if (status.kind === "ready") return relaunch();
    setStatus({ kind: "checking" });
    try {
      const update = await check();
      if (!update) return setStatus({ kind: "latest" });
      setStatus({ kind: "downloading", version: update.version });
      await update.downloadAndInstall();
      setStatus({ kind: "ready" });
    } catch (e) {
      setStatus({ kind: "error", message: e instanceof Error ? e.message : String(e) });
    }
  }

  const label = {
    idle: "Check for updates",
    checking: "Checking…",
    latest: "Check for updates",
    downloading: `Downloading ${status.kind === "downloading" ? status.version : ""}…`,
    ready: "Restart to update",
    error: "Try again",
  }[status.kind];

  const note =
    status.kind === "latest"
      ? "You're on the latest version."
      : status.kind === "error"
        ? status.message
        : null;

  const busy = status.kind === "checking" || status.kind === "downloading";

  return (
    <div className="flex flex-col items-center gap-1">
      <Button size="sm" variant="secondary" isDisabled={busy} onPress={run}>
        {label}
      </Button>
      {note && <div className="max-w-xs text-xs text-muted">{note}</div>}
    </div>
  );
}
