import { betterAuth } from "better-auth";
import { bearer } from "better-auth/plugins";
import { pool } from "./db";

/**
 * Origins allowed to call this server. Tauri's webview origin differs per
 * platform — `tauri://localhost` on macOS and Linux, `http://tauri.localhost`
 * on Windows — and 1420 is `bun run dev` in a plain browser.
 */
export const ALLOWED_ORIGINS = [
  "tauri://localhost",
  "http://tauri.localhost",
  "http://localhost:1420",
];

export const auth = betterAuth({
  database: pool,
  baseURL: process.env.BETTER_AUTH_URL ?? "http://localhost:3000",
  secret: process.env.BETTER_AUTH_SECRET,
  emailAndPassword: { enabled: true },
  // The desktop app is not a browser origin, so cookies are the wrong tool.
  // Sign-in returns the token in a `set-auth-token` header and every later
  // request sends `Authorization: Bearer …`.
  plugins: [bearer()],
  trustedOrigins: ALLOWED_ORIGINS,
});

/** The caller's user id, or null when unauthenticated. */
export async function userIdFrom(req: Request): Promise<string | null> {
  const session = await auth.api.getSession({ headers: req.headers });
  return session?.user?.id ?? null;
}
