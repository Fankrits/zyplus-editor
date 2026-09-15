import { createAuthClient } from "better-auth/react";
import { emailOTPClient } from "better-auth/client/plugins";
import { isTauri } from "@tauri-apps/api/core";
import { fetch as tauriFetch } from "@tauri-apps/plugin-http";
import { load, type Store } from "@tauri-apps/plugin-store";

/**
 * Release builds talk to the hosted API; `tauri dev` and `bun run dev` talk to the
 * local server. `VITE_ZYPLUS_API_URL` overrides either. A new host also has to be
 * added to the http allowlist in `src-tauri/capabilities/default.json`, or the
 * packaged app refuses the request before it leaves the machine.
 */
export const API_URL =
  import.meta.env.VITE_ZYPLUS_API_URL ??
  (import.meta.env.PROD ? "https://api-production-84b6.up.railway.app" : "http://localhost:3000");

/**
 * Requests go through Rust in the packaged app, not the webview.
 *
 * A Tauri window is served from a custom scheme (`tauri://localhost` on macOS),
 * and WKWebView will not make a cleartext HTTP request from one — it fails the
 * load before anything reaches the network, reporting only "Load failed". The
 * http plugin runs the request in Rust instead, which also sidesteps CORS
 * entirely; the server's CORS headers are then only for `bun run dev` in a
 * browser. Allowed URLs are pinned in `src-tauri/capabilities/default.json`.
 */
type Fetch = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

export const apiFetch: Fetch = isTauri()
  ? (input, init) => tauriFetch(input as string | URL | Request, init)
  : (input, init) => globalThis.fetch(input, init);

const TOKEN_KEY = "token";
const STORE_FILE = "auth.json";

/**
 * Better Auth's token accessor is synchronous but the Tauri store is not, so the
 * token is mirrored here and read from disk once, before React mounts.
 */
let memToken = "";

let storePromise: Promise<Store> | null = null;
function tokenStore(): Promise<Store> {
  storePromise ??= load(STORE_FILE, { autoSave: true });
  return storePromise;
}

/**
 * Outside Tauri (`bun run dev` in a browser) there is no store plugin, so the
 * token falls back to localStorage — the same dev-only shim `lib/fs.ts` uses for
 * the filesystem. In the packaged app it always lands in the app data directory,
 * out of reach of a script injected through rendered Markdown.
 */
async function readStoredToken(): Promise<string> {
  if (!isTauri()) return localStorage.getItem("zyplus:auth-token") ?? "";
  try {
    return (await (await tokenStore()).get<string>(TOKEN_KEY)) ?? "";
  } catch (err) {
    console.warn("Failed to read the stored auth token:", err);
    return "";
  }
}

async function writeStoredToken(token: string): Promise<void> {
  if (!isTauri()) {
    if (token) localStorage.setItem("zyplus:auth-token", token);
    else localStorage.removeItem("zyplus:auth-token");
    return;
  }
  try {
    const store = await tokenStore();
    if (token) await store.set(TOKEN_KEY, token);
    else await store.delete(TOKEN_KEY);
    await store.save();
  } catch (err) {
    console.warn("Failed to persist the auth token:", err);
  }
}

async function setToken(token: string): Promise<void> {
  memToken = token;
  await writeStoredToken(token);
}

export const authClient = createAuthClient({
  baseURL: API_URL,
  plugins: [emailOTPClient()],
  fetchOptions: {
    auth: { type: "Bearer", token: () => memToken },
    customFetchImpl: apiFetch,
    // Better Auth defaults to `credentials: "include"` for its cookies. This app
    // authenticates with the bearer token alone, and in `bun run dev` a
    // credentialed cross-origin request is refused by CORS outright, which made
    // every sign-in in the browser build fail as "can't reach the server".
    credentials: "omit",
  },
});

export const useSession = authClient.useSession;

/** Loads the persisted token. Call once before rendering, so the first session check has it. */
export async function initAuth(): Promise<void> {
  memToken = await readStoredToken();
}

export function isSignedIn(): boolean {
  return memToken !== "";
}

/**
 * A server that is not listening makes Better Auth *throw* rather than return an
 * error, and the message it throws ("Unable to connect. Is the computer able to
 * access the url?") names neither the server nor the problem. Naming the URL is
 * what turns it into something the reader can act on — a developer sees that
 * their local API is down, a user sees which service is unreachable.
 */
function describeFailure(err: unknown): Error {
  const detail = err instanceof Error ? err.message : String(err);
  if (/connect|network|fetch failed|failed to fetch|load failed/i.test(detail)) {
    return new Error(`Can't reach the sync server at ${API_URL}. It may be offline.`);
  }
  return err instanceof Error ? err : new Error(detail);
}

/**
 * Better Auth returns the bearer token in a header rather than the body, and only
 * when it issued a session. Responses that did not — changing a password without
 * signing other devices out, say — carry no header, and must leave the current
 * token alone rather than overwrite it with nothing and sign the user out.
 */
function captureToken(ctx: { response: Response }): Promise<void> {
  const token = ctx.response.headers.get("set-auth-token");
  return token ? setToken(token) : Promise.resolve();
}

/**
 * Every credential call funnels through here so a dead server is reported the
 * same way everywhere, and so a Better Auth error code stays machine-readable
 * instead of being flattened into a message string the caller has to re-parse.
 */
type AuthResult = { code: string | null };

async function call(
  run: () => Promise<{ error?: { message?: string; code?: string } | null }>,
  fallback: string,
  tolerate: string[] = [],
): Promise<AuthResult> {
  const { error } = await run().catch((err) => {
    throw describeFailure(err);
  });
  if (!error) return { code: null };
  const code = error.code ?? null;
  if (code && tolerate.includes(code)) return { code };
  throw new Error(error.message ?? fallback);
}

/**
 * Signs in, or reports that the account still needs its emailed code. The server
 * rejects an unverified sign-in and re-sends the code in the same breath, so the
 * caller's job is only to show the code field.
 */
export async function signIn(
  email: string,
  password: string,
): Promise<{ needsVerification: boolean }> {
  const { code } = await call(
    () => authClient.signIn.email({ email, password }, { onSuccess: captureToken }),
    "Sign in failed",
    ["EMAIL_NOT_VERIFIED"],
  );
  return { needsVerification: code === "EMAIL_NOT_VERIFIED" };
}

/**
 * Creates the account and sends the first code. There is deliberately no session
 * yet: the server withholds one until the address is confirmed, which is also why
 * it answers the same way whether or not the email was already taken.
 */
export async function signUp(email: string, password: string, name: string): Promise<void> {
  await call(() => authClient.signUp.email({ email, password, name }), "Sign up failed");
}

/** Confirms the address with the emailed code. Succeeding signs the user in. */
export async function verifyEmail(email: string, otp: string): Promise<void> {
  await call(
    () => authClient.emailOtp.verifyEmail({ email, otp }, { onSuccess: captureToken }),
    "That code didn't work",
  );
}

/** Sends a fresh code. Used for "resend" and to start a password reset. */
export async function sendCode(
  email: string,
  type: "email-verification" | "forget-password",
): Promise<void> {
  await call(
    () => authClient.emailOtp.sendVerificationOtp({ email, type }),
    "Could not send the code",
  );
}

/**
 * Sets a new password from an emailed code. The server answers identically for
 * an unknown address, so this never reveals who has an account.
 */
export async function resetPassword(
  email: string,
  otp: string,
  password: string,
): Promise<void> {
  await call(
    () => authClient.emailOtp.resetPassword({ email, otp, password }),
    "Could not reset your password",
  );
}

export async function signOut(): Promise<void> {
  // Drop the local token even if the server call fails — otherwise "sign out"
  // while offline would leave the app authenticated.
  try {
    await authClient.signOut();
  } finally {
    await setToken("");
  }
}

/* ------------------------------------------------------------------ *
 * Managing a signed-in account                                        *
 * ------------------------------------------------------------------ */

/**
 * Makes `useSession` refetch. Better Auth does this for its own account routes
 * but not for the email-otp plugin's, nor after the token is dropped locally.
 */
function refreshSession(): void {
  authClient.$store.notify("$sessionSignal");
}

/** Better Auth refreshes the session itself after `/update-user`. */
export async function updateName(name: string): Promise<void> {
  await call(() => authClient.updateUser({ name }), "Could not update your name");
}

/**
 * With `signOutOthers`, the server deletes every session — this one included —
 * and issues a fresh token, which has to be captured or this device is signed
 * out along with the rest.
 */
export async function changePassword(
  currentPassword: string,
  newPassword: string,
  signOutOthers: boolean,
): Promise<void> {
  await call(
    () =>
      authClient.changePassword(
        { currentPassword, newPassword, revokeOtherSessions: signOutOthers },
        { onSuccess: captureToken },
      ),
    "Could not change your password",
  );
}

/**
 * The signed-in "forgot password" path. A reset revokes every session, so the
 * token this device holds is dead the moment it succeeds; signing straight back
 * in with the new password is what keeps the user where they were.
 */
export async function resetPasswordAndSignIn(
  email: string,
  otp: string,
  password: string,
): Promise<void> {
  await resetPassword(email, otp, password);
  await setToken("");
  try {
    await signIn(email, password);
  } finally {
    refreshSession();
  }
}

/**
 * Step two of changing email, after a code reached the *current* address: that
 * code authorises the change and triggers a second one to the new address.
 * An address already in use gets no mail, and the answer does not say so.
 */
export async function requestEmailChange(newEmail: string, currentEmailOtp: string): Promise<void> {
  await call(
    () => authClient.emailOtp.requestEmailChange({ newEmail, otp: currentEmailOtp }),
    "Could not start the email change",
  );
}

/** Step three: the code from the new inbox. Sessions stay signed in. */
export async function confirmEmailChange(newEmail: string, otp: string): Promise<void> {
  await call(
    () => authClient.emailOtp.changeEmail({ newEmail, otp }),
    "Could not change your email",
  );
  refreshSession();
}

export async function countOtherDevices(): Promise<number> {
  const res = await authFetch("/api/account/sessions");
  if (!res.ok) throw new Error(`Could not load your devices (${res.status})`);
  return ((await res.json()) as { others: number }).others;
}

export async function signOutOtherDevices(): Promise<void> {
  await call(() => authClient.revokeOtherSessions(), "Could not sign out your other devices");
}

/**
 * Removes the account and every synced note with it. Files on this machine are
 * left exactly where they are — sync only ever mirrored them.
 */
export async function deleteAccount(password: string): Promise<void> {
  await call(() => authClient.deleteUser({ password }), "Could not delete your account");
  await setToken("");
  refreshSession();
}

/**
 * `fetch` against the notes API with the bearer token attached. Throws when the
 * caller is signed out, so sync never silently no-ops against an anonymous server.
 */
export async function authFetch(path: string, init: RequestInit = {}): Promise<Response> {
  if (!memToken) throw new Error("Not signed in");
  const headers = new Headers(init.headers);
  headers.set("Authorization", `Bearer ${memToken}`);
  if (init.body && !headers.has("Content-Type")) headers.set("Content-Type", "application/json");
  try {
    const res = await apiFetch(`${API_URL}${path}`, { ...init, headers });
    // An expired or revoked session leaves this token on disk and the session
    // hook still holding the user it last fetched, so sync keeps retrying and the
    // account panel keeps saying "signed in". Dropping the token and nudging the
    // hook to refetch turns that into a plain signed-out state, which also stops
    // sync, because sync restarts off the session's user id.
    if (res.status === 401 && memToken) {
      await setToken("");
      authClient.$store.notify("$sessionSignal");
    }
    return res;
  } catch (err) {
    throw describeFailure(err);
  }
}
