import { useState } from "react";
import { Button, Input, Label, TextField } from "@heroui/react";
import {
  API_URL,
  resetPassword,
  sendCode,
  signIn,
  signOut,
  signUp,
  useSession,
  verifyEmail,
} from "../lib/auth";
import { syncNow, useSyncStatus } from "../lib/sync";

/**
 * Signing in and signing up both end at `verify`, and forgetting a password ends
 * at `reset`; both of those are "we emailed you a code" screens. Keeping them as
 * explicit steps rather than booleans is what stops the form from rendering a
 * code field next to a password field that no longer applies.
 */
type Step = "signin" | "signup" | "verify" | "forgot" | "reset";

function describeLastSync(at: number | null): string {
  if (at === null) return "Not synced yet";
  const minutes = Math.floor((Date.now() - at) / 60000);
  if (minutes < 1) return "Synced just now";
  if (minutes < 60) return `Synced ${minutes}m ago`;
  return `Synced ${new Date(at).toLocaleString()}`;
}

/**
 * Account panel for the settings modal. Signed out it walks the credential
 * steps; signed in it shows who you are and lets you leave. Sync itself is
 * elsewhere — this only decides whether there is a user for it to sync as.
 */
export function AccountSection() {
  const { data: session, isPending } = useSession();
  const sync = useSyncStatus();
  const [step, setStep] = useState<Step>("signin");
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [code, setCode] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [isBusy, setIsBusy] = useState(false);

  /** Every step's submit shares this: clear the banners, run, report the failure. */
  const attempt = async (run: () => Promise<void>) => {
    setError(null);
    setNotice(null);
    setIsBusy(true);
    try {
      await run();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsBusy(false);
    }
  };

  const go = (next: Step) => {
    setStep(next);
    setError(null);
    setNotice(null);
    setCode("");
  };

  const submit = () =>
    attempt(async () => {
      if (step === "signin") {
        const { needsVerification } = await signIn(email, password);
        setPassword("");
        if (needsVerification) {
          setStep("verify");
          setNotice(`Confirm your email first — we sent a code to ${email}.`);
        }
        return;
      }

      if (step === "signup") {
        await signUp(email, password, name || email);
        setPassword("");
        setStep("verify");
        setNotice(`We sent a code to ${email}.`);
        return;
      }

      if (step === "verify") {
        // Succeeding signs the user in, so there is no follow-up step: the
        // session hook flips this whole panel to the signed-in view.
        await verifyEmail(email, code);
        return;
      }

      if (step === "forgot") {
        await sendCode(email, "forget-password");
        setStep("reset");
        // Deliberately worded so it reads the same whether or not the address
        // has an account — the server does not say, and neither should this.
        setNotice(`If ${email} has an account, a code is on its way.`);
        return;
      }

      await resetPassword(email, code, password);
      setPassword("");
      setStep("signin");
      setNotice("Password changed. Sign in with your new one.");
    });

  const resend = () =>
    attempt(async () => {
      await sendCode(email, step === "reset" ? "forget-password" : "email-verification");
      setNotice(`Sent another code to ${email}.`);
    });

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
          <Button size="sm" variant="secondary" isDisabled={isBusy} onPress={() => void attempt(signOut)}>
            Sign out
          </Button>
        </div>
      </div>
    );
  }

  const needsCode = step === "verify" || step === "reset";
  const needsPassword = step === "signin" || step === "signup" || step === "reset";
  // The address is fixed once a code has been sent to it: editing it here would
  // check the code against an account the code was never issued for.
  const emailIsLocked = needsCode;

  const submitLabel = {
    signin: "Sign in",
    signup: "Create account",
    verify: "Confirm email",
    forgot: "Send code",
    reset: "Set new password",
  }[step];

  const canSubmit =
    !isBusy &&
    Boolean(email) &&
    (!needsPassword || Boolean(password)) &&
    (!needsCode || Boolean(code));

  return (
    <form
      className="flex flex-col gap-4"
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
    >
      <p className="text-xs text-muted">
        {needsCode
          ? "Enter the 6-digit code we emailed you. It expires in 10 minutes."
          : step === "forgot"
            ? "We'll email you a code to set a new password."
            : "Sign in to sync your Zyplus folder across devices. Your files stay on disk either way."}
      </p>

      {step === "signup" && (
        <TextField value={name} onChange={setName} autoComplete="name">
          <Label>Name</Label>
          <Input placeholder="Your name" />
        </TextField>
      )}

      <TextField
        value={email}
        onChange={setEmail}
        type="email"
        isRequired
        isDisabled={emailIsLocked}
        autoComplete="email"
      >
        <Label>Email</Label>
        <Input placeholder="you@example.com" />
      </TextField>

      {needsCode && (
        <TextField value={code} onChange={setCode} isRequired autoComplete="one-time-code">
          <Label>Code</Label>
          <Input placeholder="123456" inputMode="numeric" />
        </TextField>
      )}

      {needsPassword && (
        <TextField
          value={password}
          onChange={setPassword}
          type="password"
          isRequired
          autoComplete={step === "signin" ? "current-password" : "new-password"}
        >
          <Label>{step === "reset" ? "New password" : "Password"}</Label>
          <Input placeholder="At least 8 characters" />
        </TextField>
      )}

      {error && <div className="text-xs text-danger">{error}</div>}
      {notice && !error && <div className="text-xs text-muted">{notice}</div>}

      <div className="flex flex-wrap items-center gap-3">
        <Button type="submit" size="sm" isDisabled={!canSubmit}>
          {submitLabel}
        </Button>

        {needsCode && (
          <LinkButton onClick={() => void resend()} isDisabled={isBusy}>
            Resend code
          </LinkButton>
        )}

        {step === "signin" && (
          <>
            <LinkButton onClick={() => go("signup")}>Create an account</LinkButton>
            <LinkButton onClick={() => go("forgot")}>Forgot password?</LinkButton>
          </>
        )}

        {step === "signup" && (
          <LinkButton onClick={() => go("signin")}>I already have an account</LinkButton>
        )}

        {step !== "signin" && step !== "signup" && (
          <LinkButton onClick={() => go("signin")}>Back to sign in</LinkButton>
        )}
      </div>
    </form>
  );
}

function LinkButton({
  onClick,
  isDisabled,
  children,
}: {
  onClick: () => void;
  isDisabled?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      disabled={isDisabled}
      className="text-xs text-muted underline-offset-2 hover:text-foreground hover:underline disabled:opacity-50"
      onClick={onClick}
    >
      {children}
    </button>
  );
}
