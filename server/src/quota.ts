/**
 * How much every synced note in one account may add up to, in bytes of UTF-8.
 * The whole database lives on one 5 GB volume shared by every account, so
 * without a cap a single account could fill it and stop sync for everyone.
 *
 * This file imports nothing, so the root test suite can check the rule without
 * installing the server's dependencies.
 */
export const STORAGE_LIMIT_BYTES = 50 * 1024 * 1024;

/**
 * Whether saving a note would take the account over its limit. Only growth is
 * refused: a note that stays the same size or shrinks always saves, so an account
 * that is already over (after the limit was lowered, say) can still edit its way
 * back under instead of having every save rejected.
 */
export function exceedsQuota({
  otherNotesBytes,
  oldBytes,
  newBytes,
  limit = STORAGE_LIMIT_BYTES,
}: {
  /** Everything the account stores except the note being saved. */
  otherNotesBytes: number;
  /** The note's stored size before this save; 0 for a new note. */
  oldBytes: number;
  newBytes: number;
  limit?: number;
}): boolean {
  return newBytes > oldBytes && otherNotesBytes + newBytes > limit;
}
