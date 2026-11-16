-- UI mail and draft rows are untouched. A request-scoped round remembers unfinished folders.
CREATE TABLE imap_round_progress (
  account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
  generation TEXT NOT NULL,
  request_version INTEGER NOT NULL,
  targets_json TEXT NOT NULL,
  pending_json TEXT NOT NULL
);
CREATE TABLE imap_flag_progress (
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  folder_key TEXT NOT NULL,
  generation TEXT NOT NULL,
  request_version INTEGER NOT NULL,
  uid_validity INTEGER NOT NULL,
  last_uid INTEGER NOT NULL,
  PRIMARY KEY(account_id, folder_key)
);
