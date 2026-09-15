import { useEffect, useState } from "react";
import { Button, Input, Label, Switch, TextField } from "@heroui/react";
import {
  API_URL,
  changePassword,
  confirmEmailChange,
  countOtherDevices,
  deleteAccount,
  requestEmailChange,
  resetPasswordAndSignIn,
  sendCode,
  signOut,
  signOutOtherDevices,
  updateName,
} from "../lib/auth";
import { syncNow, useSyncStatus } from "../lib/sync";
import { Field } from "./Field";

/**
 * Which inline editor is open. Only one at a time: two half-filled password
 * forms side by side is how someone types the new password into the wrong one.
 */
type Editor =
  | "name"
  | "email-new"
  | "email-current-code"
  | "email-new-code"
  | "password"
  | "forgot"
  | "delete"
  | null;

/** The row a success message belongs next to; the panel scrolls, so a banner would be missed. */
type Area = "name" | "email" | "password" | "devices";

function describeLastSync(at: number | null): string {
  if (at === null) return "Not synced yet";
  const minutes = Math.floor((Date.now() - at) / 60000);
  if (minutes < 1) return "Synced just now";
  if (minutes < 60) return `Synced ${minutes}m ago`;
  return `Synced ${new Date(at).toLocaleString()}`;
}

function describeDevices(others: number | null): string {
  if (others === null) return "Checking…";
  if (others === 0) return "Only this device is signed in.";
  return `Signed in on ${others} other ${others === 1 ? "device" : "devices"}.`;
}

/** The signed-in half of the account panel: profile, sync, security, and deletion. */
export function AccountSettings({ user }: { user: { name: string; email: string } }) {
  const sync = useSyncStatus();
  const [editor, setEditor] = useState<Editor>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<{ at: Area; text: string } | null>(null);
  const [isBusy, setIsBusy] = useState(false);

  const [name, setName] = useState(user.name);
  const [newEmail, setNewEmail] = useState("");
  const [code, setCode] = useState("");
  const [currentPassword, setCurrentPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [signOutOthers, setSignOutOthers] = useState(true);
  const [otherDevices, setOtherDevices] = useState<number | null>(null);

  const refreshDevices = () =>
    countOtherDevices().then(setOtherDevices, () => setOtherDevices(null));

  useEffect(() => {
    void refreshDevices();
  }, []);

  /** Opens an editor with clean fields; passwords and codes never carry over. */
  const open = (next: Editor) => {
    setEditor(next);
    setError(null);
    setNotice(null);
    setCode("");
    setCurrentPassword("");
    setNewPassword("");
    if (next === "name") setName(user.name);
    if (next === "email-new") setNewEmail("");
  };

  const attempt = async (run: () => Promise<void>) => {
    setError(null);
    setIsBusy(true);
    try {
      await run();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsBusy(false);
    }
  };

  /** Closes the editor and leaves a message beside the row the user was using. */
  const done = (at: Area, text: string) => {
    open(null);
    setNotice({ at, text });
  };

  const noticeFor = (at: Area) =>
    notice?.at === at && (
      <div role="status" className="text-xs text-accent">
        {notice.text}
      </div>
    );

  const saveName = () =>
    attempt(async () => {
      await updateName(name.trim());
      done("name", "Name updated.");
    });

  // Changing email is three short steps, because both inboxes have to prove
  // they are the account owner's before the address moves.
  const sendCurrentEmailCode = () =>
    attempt(async () => {
      await sendCode(user.email, "email-verification");
      setEditor("email-current-code");
      setCode("");
    });

  const sendNewEmailCode = () =>
    attempt(async () => {
      await requestEmailChange(newEmail.trim(), code);
      setEditor("email-new-code");
      setCode("");
    });

  const finishEmailChange = () =>
    attempt(async () => {
      await confirmEmailChange(newEmail.trim(), code);
      done("email", `Your email is now ${newEmail.trim()}.`);
    });

  const savePassword = () =>
    attempt(async () => {
      await changePassword(currentPassword, newPassword, signOutOthers);
      done(
        "password",
        signOutOthers
          ? "Password changed. Your other devices were signed out."
          : "Password changed.",
      );
      if (signOutOthers) setOtherDevices(0);
    });

  const startForgot = () =>
    attempt(async () => {
      open("forgot");
      await sendCode(user.email, "forget-password");
    });

  const finishForgot = () =>
    attempt(async () => {
      await resetPasswordAndSignIn(user.email, code, newPassword);
      done("password", "Password reset. Your other devices were signed out.");
      setOtherDevices(0);
    });

  const signOutOthersNow = () =>
    attempt(async () => {
      await signOutOtherDevices();
      setOtherDevices(0);
      setNotice({ at: "devices", text: "Your other devices were signed out." });
    });

  const confirmDelete = () =>
    attempt(async () => {
      // Succeeding drops the session, and the panel unmounts into the sign-in form.
      await deleteAccount(currentPassword);
    });

  const codeField = (label: string) => (
    <TextField value={code} onChange={setCode} isRequired autoComplete="one-time-code">
      <Label>{label}</Label>
      <Input placeholder="123456" inputMode="numeric" />
    </TextField>
  );

  const newPasswordField = (
    <TextField
      value={newPassword}
      onChange={setNewPassword}
      type="password"
      isRequired
      autoComplete="new-password"
    >
      <Label>New password</Label>
      <Input placeholder="At least 8 characters" />
    </TextField>
  );

  /** The frame every inline editor shares: fields, error, and a submit/cancel row. */
  const form = (
    submitLabel: string,
    canSubmit: boolean,
    onSubmit: () => void,
    children: React.ReactNode,
    variant: "primary" | "danger" = "primary",
  ) => (
    <form
      className="flex flex-col gap-3 rounded-lg border border-border p-3"
      onSubmit={(e) => {
        e.preventDefault();
        if (canSubmit && !isBusy) onSubmit();
      }}
    >
      {children}
      {error && <div className="text-xs text-danger">{error}</div>}
      <div className="flex items-center gap-2">
        <Button type="submit" size="sm" variant={variant} isDisabled={!canSubmit || isBusy}>
          {submitLabel}
        </Button>
        <Button size="sm" variant="ghost" isDisabled={isBusy} onPress={() => open(null)}>
          Cancel
        </Button>
      </div>
    </form>
  );

  const row = (value: string, action: React.ReactNode) => (
    <div className="flex items-center gap-3">
      <span className="min-w-0 flex-1 truncate text-foreground" title={value}>
        {value}
      </span>
      {action}
    </div>
  );

  return (
    <div className="flex flex-col gap-6">
      <section className="flex flex-col gap-4">
        <SectionTitle>Profile</SectionTitle>

        <Field label="Name">
          {editor === "name"
            ? form(
                "Save",
                name.trim() !== "" && name.trim() !== user.name,
                saveName,
                <TextField
                  value={name}
                  onChange={setName}
                  isRequired
                  autoComplete="name"
                  aria-label="Name"
                  autoFocus
                >
                  <Input />
                </TextField>,
              )
            : row(
                user.name,
                <Button size="sm" variant="secondary" onPress={() => open("name")}>
                  Edit
                </Button>,
              )}
          {noticeFor("name")}
        </Field>

        <Field
          label="Email"
          hint={editor?.startsWith("email") ? undefined : "Used to sign in and to send you codes."}
        >
          {editor === "email-new" &&
            form(
              "Continue",
              newEmail.includes("@") && newEmail.trim() !== user.email,
              sendCurrentEmailCode,
              <>
                <p className="text-xs text-muted">
                  We'll send a code to {user.email} first, to confirm it's you.
                </p>
                <TextField
                  value={newEmail}
                  onChange={setNewEmail}
                  type="email"
                  isRequired
                  autoComplete="email"
                  autoFocus
                >
                  <Label>New email</Label>
                  <Input placeholder="you@example.com" />
                </TextField>
              </>,
            )}
          {editor === "email-current-code" &&
            form(
              "Send code to new email",
              code.trim() !== "",
              sendNewEmailCode,
              <>
                <p className="text-xs text-muted">Enter the code we sent to {user.email}.</p>
                {codeField("Code from your current email")}
              </>,
            )}
          {editor === "email-new-code" &&
            form(
              "Change email",
              code.trim() !== "",
              finishEmailChange,
              <>
                <p className="text-xs text-muted">
                  If {newEmail.trim()} isn't already used by another account, a code is on its way
                  there.
                </p>
                {codeField("Code from your new email")}
              </>,
            )}
          {!editor?.startsWith("email") &&
            row(
              user.email,
              <Button size="sm" variant="secondary" onPress={() => open("email-new")}>
                Change
              </Button>,
            )}
          {noticeFor("email")}
        </Field>
      </section>

      <section className="flex flex-col gap-4">
        <SectionTitle>Sync</SectionTitle>
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
      </section>

      <section className="flex flex-col gap-4">
        <SectionTitle>Security</SectionTitle>

        <Field label="Password">
          {editor === "password" &&
            form(
              "Change password",
              currentPassword !== "" && newPassword !== "",
              savePassword,
              <>
                <TextField
                  value={currentPassword}
                  onChange={setCurrentPassword}
                  type="password"
                  isRequired
                  autoComplete="current-password"
                  autoFocus
                >
                  <Label>Current password</Label>
                  <Input />
                </TextField>
                {newPasswordField}
                <Switch isSelected={signOutOthers} onChange={setSignOutOthers}>
                  <Switch.Content>
                    <Switch.Control>
                      <Switch.Thumb />
                    </Switch.Control>
                    <Label>Sign out my other devices</Label>
                  </Switch.Content>
                </Switch>
              </>,
            )}
          {editor === "forgot" &&
            form(
              "Reset password",
              code.trim() !== "" && newPassword !== "",
              finishForgot,
              <>
                <p className="text-xs text-muted">
                  We emailed a code to {user.email}. Resetting signs out your other devices; this
                  one stays signed in.
                </p>
                {codeField("Code")}
                {newPasswordField}
                <button
                  type="button"
                  disabled={isBusy}
                  className="self-start text-xs text-muted underline-offset-2 hover:text-foreground hover:underline disabled:opacity-50"
                  onClick={() =>
                    void attempt(async () => {
                      await sendCode(user.email, "forget-password");
                      setNotice({ at: "password", text: `Sent another code to ${user.email}.` });
                    })
                  }
                >
                  Resend code
                </button>
              </>,
            )}
          {editor !== "password" && editor !== "forgot" && (
            <div className="flex flex-wrap items-center gap-3">
              <Button size="sm" variant="secondary" onPress={() => open("password")}>
                Change password
              </Button>
              <Button
                size="sm"
                variant="ghost"
                isDisabled={isBusy}
                onPress={() => void startForgot()}
              >
                Forgot password?
              </Button>
            </div>
          )}
          {noticeFor("password")}
        </Field>

        <Field label="Devices" hint={describeDevices(otherDevices)}>
          <div>
            <Button
              size="sm"
              variant="secondary"
              isDisabled={isBusy || !otherDevices}
              onPress={() => void signOutOthersNow()}
            >
              Sign out other devices
            </Button>
          </div>
          {noticeFor("devices")}
        </Field>

        <div>
          <Button
            size="sm"
            variant="secondary"
            isDisabled={isBusy}
            onPress={() => void attempt(signOut)}
          >
            Sign out of this device
          </Button>
        </div>

        {/* Errors from the actions above that have no editor of their own. */}
        {error && editor === null && <div className="text-xs text-danger">{error}</div>}
      </section>

      <section className="flex flex-col gap-4 border-t border-border pt-5">
        <SectionTitle>Delete account</SectionTitle>
        <Field
          label="Delete your account"
          hint="Removes your account and every synced note from the server. Files on this computer are not touched."
        >
          {editor === "delete" ? (
            form(
              "Delete account permanently",
              currentPassword !== "",
              confirmDelete,
              <>
                <p className="text-xs text-danger">This can't be undone.</p>
                <TextField
                  value={currentPassword}
                  onChange={setCurrentPassword}
                  type="password"
                  isRequired
                  autoComplete="current-password"
                  autoFocus
                >
                  <Label>Password</Label>
                  <Input />
                </TextField>
              </>,
              "danger",
            )
          ) : (
            <div>
              <Button size="sm" variant="danger-soft" onPress={() => open("delete")}>
                Delete account…
              </Button>
            </div>
          )}
        </Field>
      </section>
    </div>
  );
}

function SectionTitle({ children }: { children: React.ReactNode }) {
  return <h4 className="text-xs font-medium tracking-wide text-muted uppercase">{children}</h4>;
}
