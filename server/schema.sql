-- Better Auth owns its own tables (user, session, account, verification) and
-- creates them via `bun run migrate`. This file is only the note mirror.

-- One sequence shared by every user's rows. A client remembers the highest rev
-- it has pulled and asks only for rows above it, which is what makes an
-- incremental pull possible without a per-user counter table.
create sequence if not exists note_rev;

create table if not exists note (
  id         uuid        primary key default gen_random_uuid(),
  user_id    text        not null references "user"(id) on delete cascade,
  rel_path   text        not null,          -- "projects/spec.md", always forward slashes
  content    text        not null default '',
  rev        bigint      not null default nextval('note_rev'),
  updated_at timestamptz not null default now(),
  deleted_at timestamptz,                   -- tombstone, so deletes reach other devices
  unique (user_id, rel_path)
);

create index if not exists note_user_rev on note (user_id, rev);
