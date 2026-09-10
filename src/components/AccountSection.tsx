import { useState } from "react";
import { Button, Input, Label, TextField } from "@heroui/react";
import { API_URL, signIn, signOut, signUp, useSession } from "../lib/auth";
import { syncNow, useSyncStatus } from "../lib/sync";

type Mode = "signin" | "signup";

function describeLastSync(at: number | null): string {
  if (at === null) return "Not synced yet";
  const minutes = Math.floor((Date.now() - at) / 60000);
  if (minutes < 1) return "Synced just now";
  if (minutes < 60) return `Synced ${minutes}m ago`;
  return `Synced ${new Date(at).toLocaleString()}`;
}

/**
 * Account panel for the settings modal. Signed out it takes credentials; signed
 * in it shows who you are and lets you leave. Sync itself is elsewhere — this
 * only decides whether there is a user for it to sync as.
 */
export function AccountSection() {
  const { data: session, isPending } = useSession();
  const sync = useSyncStatus();
  const [mode, setMode] = useState<Mode>("signin");
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [isBusy, setIsBusy] = useState(false);

  const submit = async () => {
    setError(null);
    setIsBusy(true);
    try {
      if (mode === "signup") await signUp(email, password, name || email);
      else await signIn(email, password);
      setPassword("");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsBusy(false);
    }
  };

  const leave = async () => {
    setIsBusy(true);
    try {
      await signOut();
    } finally {
      setIsBusy(false);
    }
  };

  if (isPending) {
    return <div className="text-sm text-muted">Checking your session…</div>;
  }

  if (session?.user) {
    return (
      <div className="flex flex-col gap-5">
        <div className="flex flex-col gap-2">
          <div className="text-sm font-medium text-foreground">Signed in</div>
          <div className="truncate text-sm text-muted" title={session.user.email}>
            {session.user.email}
          </div>
        </div>
        <p className="text-xs text-muted">
          Your <span className="text-foreground">Zyplus</span> folder syncs to {API_URL}. Every
          other folder you open stays on this machine.
        </p>
        <div className="flex items-center gap-3">
          <Button
            size="sm"
            variant="secondary"
            isDisabled={sync.state === "syncing"}
            onPress={() => void syncNow()}
          >
            {sync.state === "syncing" ? "Syncing…" : "Sync now"}
          </Button>
          <span className={`text-xs ${sync.state === "error" ? "text-danger" : "text-muted"}`}>
            {sync.error ?? describeLastSync(sync.lastSyncedAt)}
          </span>
        </div>
        <div>
          <Button size="sm" variant="secondary" isDisabled={isBusy} onPress={leave}>
            Sign out
          </Button>
        </div>
      </div>
    );
  }

  return (
    <form
      className="flex flex-col gap-4"
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
    >
      <p className="text-xs text-muted">
        Sign in to sync your <span className="text-foreground">Zyplus</span> folder across
        devices. Your files stay on disk either way.
      </p>

      {mode === "signup" && (
        <TextField value={name} onChange={setName} autoComplete="name">
          <Label>Name</Label>
          <Input placeholder="Your name" />
        </TextField>
      )}

      <TextField value={email} onChange={setEmail} type="email" isRequired autoComplete="email">
        <Label>Email</Label>
        <Input placeholder="you@example.com" />
      </TextField>

      <TextField
        value={password}
        onChange={setPassword}
        type="password"
        isRequired
        autoComplete={mode === "signup" ? "new-password" : "current-password"}
      >
        <Label>Password</Label>
        <Input placeholder="At least 8 characters" />
      </TextField>

      {error && <div className="text-xs text-danger">{error}</div>}

      <div className="flex items-center gap-3">
        <Button type="submit" size="sm" isDisabled={isBusy || !email || !password}>
          {mode === "signup" ? "Create account" : "Sign in"}
        </Button>
        <button
          type="button"
          className="text-xs text-muted underline-offset-2 hover:text-foreground hover:underline"
          onClick={() => {
            setMode(mode === "signup" ? "signin" : "signup");
            setError(null);
          }}
        >
          {mode === "signup" ? "I already have an account" : "Create an account"}
        </button>
      </div>
    </form>
  );
}
