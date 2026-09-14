import { betterAuth } from "better-auth";
import { bearer, emailOTP, haveIBeenPwned } from "better-auth/plugins";
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

/**
 * Resend's own sandbox sender. It delivers only to the address that owns the
 * Resend account, which is enough to develop against but not to ship: real
 * users need a domain here with SPF and DKIM records, or the mail is spam.
 */
const MAIL_FROM = process.env.MAIL_FROM ?? "Zyplus <onboarding@resend.dev>";

/** Transactional mail over Resend's HTTP API. No SDK — it is one POST. */
async function sendMail(to: string, subject: string, text: string): Promise<void> {
  const apiKey = process.env.RESEND_API_KEY;
  // Better to fail loudly here than to let Better Auth report a sent code that
  // no mail server ever accepted, leaving the user waiting on nothing.
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
      async sendVerificationOTP({ email, otp, type }) {
        const purpose = OTP_PURPOSE[type] ?? "continue";
        await sendMail(
          email,
          `Your Zyplus code: ${otp}`,
          `Your code to ${purpose} is ${otp}\n\n` +
            `It expires in 10 minutes. If you didn't ask for it, you can ignore this email.`,
        );
      },
    }),

    // Rejects passwords known to be breached, on sign-up and on every reset.
    // Only the first five characters of the password's SHA-1 leave this server.
    haveIBeenPwned(),
  ],

  trustedOrigins: ALLOWED_ORIGINS,
});

/** The caller's user id, or null when unauthenticated. */
export async function userIdFrom(req: Request): Promise<string | null> {
  const session = await auth.api.getSession({ headers: req.headers });
  return session?.user?.id ?? null;
}
