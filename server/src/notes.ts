import { pool } from "./db";

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

/**
 * Writes `content` if the stored row is still at `baseRev` (0 meaning "the
 * client believes this does not exist yet"). The `where rev = baseRev` is the
 * entire conflict mechanism: when it matches nothing, somebody else wrote
 * first and the caller gets the winning row back to save alongside.
 */
export async function put(
  userId: string,
  relPath: string,
  content: string,
  baseRev: number,
): Promise<{ rev: number } | Conflict> {
  if (baseRev === 0) {
    const { rows } = await pool.query(
      `insert into note (user_id, rel_path, content)
            values ($1, $2, $3)
       on conflict (user_id, rel_path) do nothing
         returning rev`,
      [userId, relPath, content],
    );
    if (rows[0]) return { rev: Number(rows[0].rev) };
  } else {
    const { rows } = await pool.query(
      `update note
          set content = $3,
              rev = nextval('note_rev'),
              updated_at = now(),
              deleted_at = null
        where user_id = $1 and rel_path = $2 and rev = $4
      returning rev`,
      [userId, relPath, content, baseRev],
    );
    if (rows[0]) return { rev: Number(rows[0].rev) };
  }

  const current = await getContent(userId, relPath);
  // Losing the race to a delete leaves no row at all; treat it as an empty winner
  // so the client still resolves to a single, well-defined outcome.
  return { conflict: true, rev: current?.rev ?? 0, content: current?.content ?? "" };
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
