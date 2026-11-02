-- v4: every conversation belongs to exactly one authenticated account.
--
-- The dashboard is multi-user: /api/conversations must never return another
-- user's chats. `owner_account_id` references `auth_account.id`, which lives
-- in the auth database (a separate SQLite file beside this one), so the
-- relationship is enforced by the backend authorization layer rather than a
-- cross-database FOREIGN KEY (SQLite cannot express one). Every read, write,
-- update and delete path resolves the owner from the verified server-side
-- session and matches it against this column.
--
-- NULL means "ownership cannot be established" — rows written before this
-- migration. Those rows are preserved untouched and are NEVER arbitrarily
-- reassigned: regular users do not see them, and administrators reach them
-- through /api/admin/conversations as an explicit, audited recovery path
-- (where they can be assigned to an account by hand).

ALTER TABLE conversations ADD COLUMN owner_account_id INTEGER;

-- User-scoped listing: one owner's newest conversation first. This is the
-- index behind GET /api/conversations, so the common path never scans other
-- users' rows.
CREATE INDEX IF NOT EXISTS idx_conversations_owner_updated
  ON conversations(owner_account_id, updated_at_ms DESC);
