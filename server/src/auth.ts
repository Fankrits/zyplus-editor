import { betterAuth } from "better-auth";
import { APIError, createAuthMiddleware } from "better-auth/api";
import { bearer, emailOTP, haveIBeenPwned } from "better-auth/plugins";
import { pool } from "./db";

/**
 * Origins allowed to call this server. Tauri's webview origin differs per
 * platform — `tauri://localhost` on macOS and Linux, `http://tauri.localhost`
 * on Windows — and 1420 is `bun run dev` in a plain browser. `WEB_ORIGINS` adds
 * wherever the web build is hosted, comma-separated.
 */
export const ALLOWED_ORIGINS = [
  "tauri://localhost",
  "http://tauri.localhost",
  "http://localhost:1420",
  ...(process.env.WEB_ORIGINS?.split(",").map((o) => o.trim()).filter(Boolean) ?? []),
];

/**
 * Resend's own sandbox sender. It delivers only to the address that owns the
 * Resend account, which is enough to develop against but not to ship: real
 * users need a domain here with SPF and DKIM records, or the mail is spam.
 */
const MAIL_FROM = process.env.MAIL_FROM ?? "Zyplus <onboarding@resend.dev>";

/** Transactional mail over Resend's HTTP API. No SDK — it is one POST. */
async function sendMail(to: string, subject: string, text: string): Promise<void> {
  const apiKey = process.env.RESEND_API_KEY;
  // A failure never reaches the user (see `sendVerificationOTP`), so the server log
  // is the only place a mail outage shows, and the message has to say what failed.
  if (!apiKey) throw new Error("RESEND_API_KEY is not set, so no mail can be sent");

  const res = await fetch("https://api.resend.com/emails", {
    method: "POST",
    headers: { Authorization: `Bearer ${apiKey}`, "Content-Type": "application/json" },
    body: JSON.stringify({ from: MAIL_FROM, to, subject, text }),
  });
  if (!res.ok) {
    throw new Error(`Resend rejected the message (${res.status}): ${await res.text()}`);
  }
}

/** What the reader is being asked to do, for the body of the code email. */
const OTP_PURPOSE: Record<string, string> = {
  "sign-in": "sign in to Zyplus",
  "email-verification": "confirm your email address",
  "forget-password": "reset your Zyplus password",
  "change-email": "change your Zyplus email address",
};

export const auth = betterAuth({
  database: pool,
  baseURL: process.env.BETTER_AUTH_URL ?? "http://localhost:3000",
  secret: process.env.BETTER_AUTH_SECRET,

  emailAndPassword: {
    enabled: true,
    // Without this an address can be claimed by someone who does not own it,
    // locking its real owner out of ever registering.
    requireEmailVerification: true,
    // A reset is often a response to someone else having the password. Leaving
    // their session alive would hand them another month of access to the notes.
    revokeSessionsOnPasswordReset: true,
  },

  // Both flags are merged into the email-otp plugin's own `sendVerificationEmail`
  // below: plugin `init` options are folded in with `defu`, which lets the user's
  // config win, so defining `sendVerificationEmail` here would silently disable
  // the OTP override and put link-based verification back.
  // `autoSignInAfterVerification` is what makes the code a complete step: entering
  // it returns a session token, so a new account lands signed in rather than back
  // at a login form it just filled in.
  emailVerification: {
    sendOnSignUp: true,
    sendOnSignIn: true,
    autoSignInAfterVerification: true,
  },

  // Deleting cascades to every synced note (see schema.sql). Better Auth would
  // accept it with no password at all from a session under a day old, so the
  // `before` hook below makes the password mandatory; the route then checks it.
  user: { deleteUser: { enabled: true } },

  hooks: {
    before: createAuthMiddleware(async (ctx) => {
      if (ctx.path === "/delete-user" && !ctx.body?.password) {
        throw new APIError("BAD_REQUEST", {
          message: "Enter your password to delete your account",
          code: "PASSWORD_REQUIRED",
        });
      }
    }),
  },

  // A desktop app has no web page for a reset link to land on. Seven days is
  // also short for an app opened weekly, so sessions last a month instead.
  session: { expiresIn: 60 * 60 * 24 * 30 },

  // Better Auth enables rate limiting in production only. Sign-in is the endpoint
  // worth holding tighter than the 100-per-10s global default; the OTP endpoints
  // carry their own 3-per-60s limit from the plugin.
  rateLimit: {
    customRules: {
      "/sign-in/email": { window: 60, max: 5 },
      "/sign-up/email": { window: 3600, max: 10 },
      // Both check the current password, so a stolen session token must not be
      // able to guess it at the global rate.
      "/change-password": { window: 60, max: 5 },
      "/delete-user": { window: 60, max: 5 },
    },
  },

  plugins: [
    // The desktop app is not a browser origin, so cookies are the wrong tool.
    // Sign-in returns the token in a `set-auth-token` header and every later
    // request sends `Authorization: Bearer …`.
    bearer(),

    // Codes rather than links, for the same reason: the user types six digits
    // back into the window they are already looking at, with no browser detour
    // and no reset page to host.
    emailOTP({
      expiresIn: 600, // 10 minutes — long enough to switch to a mail client and back.
      // A database leak should not hand over live codes.
      storeOTP: "hashed",
      overrideDefaultEmailVerification: true,
      // Changing the address takes a code from the current inbox as well as the
      // new one. Otherwise a stolen session token could move the account to an
      // inbox its thief controls and then take it over with a password reset.
      changeEmail: { enabled: true, verifyCurrentEmail: true },
      // Not awaited. Better Auth only calls this for addresses that have an
      // account, so waiting on Resend made those requests ~200ms slower than ones
      // for unknown addresses — giving away exactly what the identical responses
      // are there to hide. Only the HTTP call is detached: Better Auth has already
      // stored the code by now, and sign-up runs inside a database transaction
      // that a detached database read would outlive.
      async sendVerificationOTP({ email, otp, type }) {
        const purpose = OTP_PURPOSE[type] ?? "continue";
        void sendMail(
          email,
          `Your Zyplus code: ${otp}`,
          `Your code to ${purpose} is ${otp}\n\n` +
            `It expires in 10 minutes. If you didn't ask for it, you can ignore this email.`,
        ).catch((err) => console.error(`Failed to send a ${type} code:`, err));
      },
    }),

    // Rejects passwords known to be breached, on sign-up and on every reset.
    // Only the first five characters of the password's SHA-1 leave this server.
    haveIBeenPwned(),
  ],

  trustedOrigins: ALLOWED_ORIGINS,

  // Rate limits are keyed by client IP. Railway's edge puts the caller's address
  // in `X-Real-IP`; Better Auth's default, `X-Forwarded-For`, is trusted only when
  // it holds a single hop, and otherwise every caller collapses into one shared
  // bucket — where one person's failed sign-ins lock everybody out. Locally the
  // header is absent and Better Auth falls back to localhost.
  advanced: { ipAddress: { ipAddressHeaders: ["x-real-ip"] } },
});

/** The caller's user id, or null when unauthenticated. */
export async function userIdFrom(req: Request): Promise<string | null> {
  const session = await auth.api.getSession({ headers: req.headers });
  return session?.user?.id ?? null;
}
