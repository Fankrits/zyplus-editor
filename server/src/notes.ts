import { pool } from "./db";
import { exceedsQuota, STORAGE_LIMIT_BYTES } from "./quota";

export interface NoteMeta {
  relPath: string;
  rev: number;
  deletedAt: string | null;
}

export interface NoteContent extends NoteMeta {
  content: string;
}

/** A conflict: the row moved past the client's `baseRev`. Carries the winner. */
export interface Conflict {
  conflict: true;
  rev: number;
  content: string;
}

/** The save would take the account past its storage limit. Nothing was written. */
export interface StorageFull {
  storageFull: true;
  used: number;
  limit: number;
}

const MAX_REL_PATH = 1024;

/**
 * Relative paths become filesystem paths on the client, so traversal is
 * rejected here rather than trusted. Postgres does not care; the client does.
 */
export function isValidRelPath(relPath: unknown): relPath is string {
  if (typeof relPath !== "string") return false;
  if (relPath.length === 0 || relPath.length > MAX_REL_PATH) return false;
  if (relPath.startsWith("/") || relPath.includes("\\")) return false;
  if (relPath.includes("\0")) return false;
  return !relPath.split("/").some((seg) => seg === "" || seg === "." || seg === "..");
}

/** Metadata for everything that changed after `since`. Content is fetched separately. */
export async function listSince(userId: string, since: number): Promise<NoteMeta[]> {
  const { rows } = await pool.query(
    `select rel_path, rev, deleted_at
       from note
      where user_id = $1 and rev > $2
      order by rev`,
    [userId, since],
  );
  return rows.map((r) => ({
    relPath: r.rel_path,
    rev: Number(r.rev),
    deletedAt: r.deleted_at ? new Date(r.deleted_at).toISOString() : null,
  }));
}

export async function getContent(userId: string, relPath: string): Promise<NoteContent | null> {
  const { rows } = await pool.query(
    `select rel_path, rev, content, deleted_at
       from note
      where user_id = $1 and rel_path = $2`,
    [userId, relPath],
  );
  const row = rows[0];
  if (!row) return null;
  return {
    relPath: row.rel_path,
    rev: Number(row.rev),
    content: row.content,
    deletedAt: row.deleted_at ? new Date(row.deleted_at).toISOString() : null,
  };
}

/** Bytes of note content the account stores. Tombstones hold no content. */
export async function usageOf(userId: string): Promise<number> {
  const { rows } = await pool.query(
    `select coalesce(sum(octet_length(content)), 0)::bigint as used from note where user_id = $1`,
    [userId],
  );
  return Number(rows[0].used);
}

/**
 * Writes `content` if the stored row is still at `baseRev` (0 meaning "the
 * client believes this does not exist yet"). The `where rev = baseRev` is the
 * entire conflict mechanism: when it matches nothing, somebody else wrote
 * first and the caller gets the winning row back to save alongside.
 *
 * The storage check and the write share one transaction under a per-account
 * lock. Without the lock, two devices uploading at once could each pass the
 * check against the same total and together go over the limit.
 */
export async function put(
  userId: string,
  relPath: string,
  content: string,
  baseRev: number,
): Promise<{ rev: number } | Conflict | StorageFull> {
  const client = await pool.connect();
  try {
    await client.query("begin");
    await client.query("select pg_advisory_xact_lock(hashtext($1))", [userId]);

    const { rows: sizeRows } = await client.query(
      `select coalesce(sum(octet_length(content)) filter (where rel_path <> $2), 0)::bigint as others,
              coalesce(sum(octet_length(content)) filter (where rel_path = $2), 0)::bigint as old
         from note
        where user_id = $1`,
      [userId, relPath],
    );
    const otherNotesBytes = Number(sizeRows[0].others);
    const oldBytes = Number(sizeRows[0].old);
    const newBytes = Buffer.byteLength(content, "utf8");
    if (exceedsQuota({ otherNotesBytes, oldBytes, newBytes })) {
      await client.query("rollback");
      return { storageFull: true, used: otherNotesBytes + oldBytes, limit: STORAGE_LIMIT_BYTES };
    }

    const { rows } =
      baseRev === 0
        ? await client.query(
            `insert into note (user_id, rel_path, content)
                  values ($1, $2, $3)
             on conflict (user_id, rel_path) do nothing
               returning rev`,
            [userId, relPath, content],
          )
        : await client.query(
            `update note
                set content = $3,
                    rev = nextval('note_rev'),
                    updated_at = now(),
                    deleted_at = null
              where user_id = $1 and rel_path = $2 and rev = $4
            returning rev`,
            [userId, relPath, content, baseRev],
          );

    if (rows[0]) {
      await client.query("commit");
      return { rev: Number(rows[0].rev) };
    }

    const { rows: current } = await client.query(
      `select rev, content from note where user_id = $1 and rel_path = $2`,
      [userId, relPath],
    );
    await client.query("commit");
    // Losing the race to a delete leaves no row at all; treat it as an empty winner
    // so the client still resolves to a single, well-defined outcome.
    return {
      conflict: true,
      rev: current[0] ? Number(current[0].rev) : 0,
      content: current[0]?.content ?? "",
    };
  } catch (err) {
    await client.query("rollback").catch(() => {});
    throw err;
  } finally {
    client.release();
  }
}

/** Tombstones the row so the delete reaches other devices. Idempotent. */
export async function softDelete(userId: string, relPath: string): Promise<{ rev: number } | null> {
  const { rows } = await pool.query(
    `update note
        set deleted_at = now(),
            content = '',
            rev = nextval('note_rev'),
            updated_at = now()
      where user_id = $1 and rel_path = $2 and deleted_at is null
    returning rev`,
    [userId, relPath],
  );
  return rows[0] ? { rev: Number(rows[0].rev) } : null;
}
