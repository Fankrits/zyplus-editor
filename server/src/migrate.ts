import { getMigrations } from "better-auth/db/migration";
import { auth } from "./auth";
import { pool } from "./db";

/**
 * Brings a database up to date: Better Auth's tables first, then the note
 * mirror that references them. Both steps are idempotent, so this runs before
 * every deploy and locally after `db:up` alike.
 *
 * It replaces a pipeline that downloaded `@better-auth/cli@latest` on each run
 * and piped schema.sql through `podman exec` — neither exists on a host, and an
 * unpinned CLI can migrate with a different Better Auth than the server runs.
 */
const { toBeCreated, toBeAdded, runMigrations } = await getMigrations(auth.options);
const pending = [...toBeCreated, ...toBeAdded].map((t) => t.table);
await runMigrations();
console.log(pending.length ? `Better Auth: migrated ${pending.join(", ")}` : "Better Auth: up to date");

// schema.sql is several statements with no parameters, which the simple query
// protocol runs as one batch — and inside one implicit transaction.
await pool.query(await Bun.file(new URL("../schema.sql", import.meta.url)).text());
console.log("Note schema: applied");

await pool.end();
