import { Pool } from "pg";

export const isProduction = process.env.NODE_ENV === "production";

/**
 * Variables a deployed server cannot run without. Each has a development default
 * or fails only when first used — a missing mail key surfaces as the first user
 * who never gets a code, a missing database URL as a quiet attempt at localhost —
 * so production checks them all up front and refuses to boot instead.
 */
if (isProduction) {
  const required = ["DATABASE_URL", "BETTER_AUTH_SECRET", "BETTER_AUTH_URL", "RESEND_API_KEY", "MAIL_FROM"];
  const missing = required.filter((name) => !process.env[name]);
  if (missing.length > 0) throw new Error(`Missing required environment variables: ${missing.join(", ")}`);
}

export const DATABASE_URL =
  process.env.DATABASE_URL ?? "postgres://zyplus:zyplus@localhost:5432/zyplus";

/** One pool for the process; Better Auth and the note routes share it. */
export const pool = new Pool({ connectionString: DATABASE_URL });
