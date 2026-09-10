import { Pool } from "pg";

export const DATABASE_URL =
  process.env.DATABASE_URL ?? "postgres://zyplus:zyplus@localhost:5432/zyplus";

/** One pool for the process; Better Auth and the note routes share it. */
export const pool = new Pool({ connectionString: DATABASE_URL });
