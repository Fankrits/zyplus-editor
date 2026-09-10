import { createAuthClient } from "better-auth/react";
import { isTauri } from "@tauri-apps/api/core";
import { fetch as tauriFetch } from "@tauri-apps/plugin-http";
import { load, type Store } from "@tauri-apps/plugin-store";

export const API_URL = import.meta.env.VITE_ZYPLUS_API_URL ?? "http://localhost:3000";

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
  fetchOptions: {
    auth: { type: "Bearer", token: () => memToken },
    customFetchImpl: apiFetch,
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

/** Better Auth returns the bearer token in a header rather than the body. */
function captureToken(ctx: { response: Response }): Promise<void> {
  return setToken(ctx.response.headers.get("set-auth-token") ?? "");
}

export async function signIn(email: string, password: string): Promise<void> {
  const { error } = await authClient
    .signIn.email({ email, password }, { onSuccess: captureToken })
    .catch((err) => {
      throw describeFailure(err);
    });
  if (error) throw new Error(error.message ?? "Sign in failed");
}

export async function signUp(email: string, password: string, name: string): Promise<void> {
  const { error } = await authClient
    .signUp.email({ email, password, name }, { onSuccess: captureToken })
    .catch((err) => {
      throw describeFailure(err);
    });
  if (error) throw new Error(error.message ?? "Sign up failed");
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
    return await apiFetch(`${API_URL}${path}`, { ...init, headers });
  } catch (err) {
    throw describeFailure(err);
  }
}
