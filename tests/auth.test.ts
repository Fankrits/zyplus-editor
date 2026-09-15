import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import {
  API_URL,
  authClient,
  authFetch,
  changePassword,
  deleteAccount,
  initAuth,
  isSignedIn,
  countOtherDevices,
  resetPasswordAndSignIn,
  sendCode,
  signIn,
} from "../src/lib/auth";

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

/** Answers successive requests in order, recording which paths were called. */
function respondInOrder(
  replies: { status?: number; body?: unknown; token?: string }[],
): string[] {
  const paths: string[] = [];
  globalThis.fetch = (async (input: RequestInfo | URL) => {
    paths.push(new URL(String(input instanceof Request ? input.url : input)).pathname);
    const { status = 200, body = {}, token } = replies[paths.length - 1] ?? {};
    const headers = new Headers({ "Content-Type": "application/json" });
    if (token) headers.set("set-auth-token", token);
    // An empty string stands for an empty body, which is what a missing route returns.
    return new Response(body === "" ? null : JSON.stringify(body), { status, headers });
  }) as unknown as typeof fetch;
  return paths;
}

const storedToken = () => localStorage.getItem("zyplus:auth-token");

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
    await expect(signIn("a@b.com", "wrong")).rejects.toThrow("Invalid email or password");
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
    await expect(authFetch("/api/notes")).rejects.toThrow("Not signed in");
  });

  it("names the server when it cannot be reached", async () => {
    await signInWith("good-token");
    globalThis.fetch = (() =>
      Promise.reject(new Error("fetch failed"))) as unknown as typeof fetch;
    await expect(authFetch("/api/notes")).rejects.toThrow(API_URL);
  });
});

describe("account management", () => {
  it("keeps this device signed in when a password change issues no new token", async () => {
    // Without signing other devices out, the server keeps the current session and
    // sends no `set-auth-token`. Treating the missing header as an empty token
    // would sign the user out of the device they just changed the password on.
    await signInWith("current-token");
    respondInOrder([{ body: { token: null } }]);

    await changePassword("old-password-1", "new-password-22", false);

    expect(isSignedIn()).toBe(true);
    expect(storedToken()).toBe("current-token");
  });

  it("adopts the replacement token when other devices are signed out", async () => {
    // Signing others out deletes every session, this one included; only the
    // token in the response keeps this device signed in.
    await signInWith("current-token");
    respondInOrder([{ body: { token: "fresh-token" }, token: "fresh-token" }]);

    await changePassword("old-password-1", "new-password-22", true);

    expect(storedToken()).toBe("fresh-token");
  });

  it("signs straight back in after a signed-in password reset", async () => {
    await signInWith("revoked-by-reset");
    const paths = respondInOrder([
      { body: { success: true } },
      { body: { token: "after-reset" }, token: "after-reset" },
    ]);

    await resetPasswordAndSignIn("a@b.com", "123456", "new-password-22");

    expect(paths.map((p) => p.replace(/^.*\/api\/auth/, ""))).toEqual([
      "/email-otp/reset-password",
      "/sign-in/email",
    ]);
    expect(storedToken()).toBe("after-reset");
  });

  it("does not leave the revoked token behind when signing back in fails", async () => {
    await signInWith("revoked-by-reset");
    respondInOrder([
      { body: { success: true } },
      { status: 429, body: { message: "Too many requests" } },
    ]);

    await expect(resetPasswordAndSignIn("a@b.com", "123456", "new-password-22")).rejects.toThrow();

    expect(isSignedIn()).toBe(false);
  });

  it("forgets the token once the account is deleted", async () => {
    await signInWith("doomed-token");
    respondInOrder([{ body: { success: true } }]);

    await deleteAccount("my-password-1");

    expect(isSignedIn()).toBe(false);
    expect(storedToken()).toBeNull();
  });

  it("keeps the token when deletion is refused", async () => {
    await signInWith("still-mine");
    respondInOrder([{ status: 400, body: { code: "INVALID_PASSWORD", message: "Invalid password" } }]);

    await expect(deleteAccount("wrong-password")).rejects.toThrow("Invalid password");

    expect(isSignedIn()).toBe(true);
  });
});

describe("error messages", () => {
  it("names an outdated server when an auth route is missing", async () => {
    // A server without the email-otp plugin answers 404 with an empty body. The
    // generic fallback ("Could not send the code") gave the user nothing to act on.
    respondInOrder([{ status: 404, body: "" }]);
    await expect(sendCode("a@b.com", "forget-password")).rejects.toThrow(
      `The sync server at ${API_URL} doesn't support this yet`,
    );
  });

  it("names an outdated server when the device count route is missing", async () => {
    await signInWith("a-token");
    respondInOrder([{ status: 404, body: "" }]);
    await expect(countOtherDevices()).rejects.toThrow("doesn't support this yet");
  });

  it("explains a rate limit instead of passing on a bare status", async () => {
    respondInOrder([{ status: 429, body: {} }]);
    await expect(sendCode("a@b.com", "forget-password")).rejects.toThrow("Too many attempts");
  });

  it("falls back to the action's own message when the server sends an empty one", async () => {
    respondInOrder([{ status: 400, body: { message: "" } }]);
    await expect(sendCode("a@b.com", "forget-password")).rejects.toThrow("Could not send the code");
  });
});
