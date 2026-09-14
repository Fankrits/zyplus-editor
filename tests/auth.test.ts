import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import { API_URL, authClient, authFetch, initAuth, isSignedIn, signIn } from "../src/lib/auth";

/**
 * `apiFetch` reads `globalThis.fetch` at call time rather than closing over it,
 * so swapping it here reaches both the raw note requests and Better Auth's own
 * client, which is wired to the same implementation.
 */
const realFetch = globalThis.fetch;

function respondWith(status: number, body: unknown): Response[] {
  const seen: Response[] = [];
  globalThis.fetch = (async () => {
    const res = new Response(JSON.stringify(body), {
      status,
      headers: { "Content-Type": "application/json" },
    });
    seen.push(res);
    return res;
  }) as unknown as typeof fetch;
  return seen;
}

async function signInWith(token: string): Promise<void> {
  localStorage.setItem("zyplus:auth-token", token);
  await initAuth();
}

beforeEach(() => {
  localStorage.clear();
});

afterEach(async () => {
  globalThis.fetch = realFetch;
  localStorage.clear();
  await initAuth();
});

describe("signIn", () => {
  it("reports an unverified account instead of failing", async () => {
    // The server rejects the sign-in *and* mails a fresh code in the same call.
    // Treating this like any other error would strand the user on a form with no
    // way to reach the code that is already in their inbox.
    respondWith(403, { code: "EMAIL_NOT_VERIFIED", message: "Email not verified" });
    expect(await signIn("a@b.com", "pw")).toEqual({ needsVerification: true });
  });

  it("throws on a wrong password rather than asking for a code", async () => {
    respondWith(401, {
      code: "INVALID_EMAIL_OR_PASSWORD",
      message: "Invalid email or password",
    });
    expect(signIn("a@b.com", "wrong")).rejects.toThrow("Invalid email or password");
  });
});

describe("authFetch", () => {
  it("drops a token the server no longer accepts", async () => {
    await signInWith("stale-token");
    expect(isSignedIn()).toBe(true);

    respondWith(401, { error: "Unauthorized" });
    const res = await authFetch("/api/notes");

    // Without this the session hook shows "signed out" while sync keeps retrying
    // with a token the server will never accept again.
    expect(res.status).toBe(401);
    expect(isSignedIn()).toBe(false);
    expect(localStorage.getItem("zyplus:auth-token")).toBeNull();
  });

  it("tells the session hook to refetch, so the UI and sync stop too", async () => {
    await signInWith("stale-token");
    const signal = authClient.$store.atoms.$sessionSignal;
    const before = signal.get();

    respondWith(401, { error: "Unauthorized" });
    await authFetch("/api/notes");

    expect(signal.get()).not.toBe(before);
  });

  it("keeps the token when the request succeeds", async () => {
    await signInWith("good-token");
    respondWith(200, { notes: [] });

    expect((await authFetch("/api/notes")).status).toBe(200);
    expect(isSignedIn()).toBe(true);
  });

  it("refuses to call the API at all when signed out", async () => {
    await initAuth();
    expect(authFetch("/api/notes")).rejects.toThrow("Not signed in");
  });

  it("names the server when it cannot be reached", async () => {
    await signInWith("good-token");
    globalThis.fetch = (() =>
      Promise.reject(new Error("fetch failed"))) as unknown as typeof fetch;
    expect(authFetch("/api/notes")).rejects.toThrow(API_URL);
  });
});
