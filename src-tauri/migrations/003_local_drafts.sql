-- Existing local drafts start at revision 1. Revision 0 always means a new draft.
ALTER TABLE drafts ADD COLUMN revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0);
CREATE INDEX drafts_account_updated ON drafts(account_id, updated_at DESC, id DESC);
-- One-time import receipts prevent old WebView data from resurrecting deleted drafts.
CREATE TABLE local_imports (
  source_key TEXT PRIMARY KEY,
  completed_at TEXT NOT NULL
);
