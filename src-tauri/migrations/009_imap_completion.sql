ALTER TABLE messages ADD COLUMN raw_cache_key TEXT;
ALTER TABLE messages ADD COLUMN original_remote_id TEXT;
ALTER TABLE messages ADD COLUMN attachments_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE messages ADD COLUMN remote_missing INTEGER NOT NULL DEFAULT 0 CHECK(remote_missing IN (0,1));
ALTER TABLE message_recovery ADD COLUMN auto_continue INTEGER NOT NULL DEFAULT 0 CHECK(auto_continue IN (0,1));
CREATE TABLE imap_settings (
  account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
  sync_all_folders INTEGER NOT NULL DEFAULT 1 CHECK(sync_all_folders IN (0,1)),
  history_enabled INTEGER NOT NULL DEFAULT 0 CHECK(history_enabled IN (0,1)),
  auto_sync_seconds INTEGER NOT NULL DEFAULT 0 CHECK(auto_sync_seconds=0 OR auto_sync_seconds BETWEEN 60 AND 86400)
);
INSERT INTO imap_settings(account_id) SELECT id FROM accounts WHERE provider='imap' AND COALESCE(json_extract(CASE WHEN json_valid(profile_json) THEN profile_json ELSE '{}' END,'$.status'),'')!='demo';
CREATE TABLE imap_folder_state (
  account_id TEXT NOT NULL,
  remote_id TEXT NOT NULL,
  available INTEGER NOT NULL CHECK(available IN (0,1)),
  PRIMARY KEY(account_id,remote_id),
  FOREIGN KEY(account_id,remote_id) REFERENCES folders(account_id,remote_id) ON DELETE CASCADE
);
CREATE TABLE imap_history (
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  folder TEXT NOT NULL,
  generation TEXT NOT NULL,
  uid_validity INTEGER NOT NULL CHECK(uid_validity>0),
  before_uid INTEGER NOT NULL CHECK(before_uid>0),
  PRIMARY KEY(account_id,folder)
);
CREATE TABLE imap_sync_requests (
  account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
  generation TEXT NOT NULL,
  state TEXT NOT NULL CHECK(state IN ('queued','running','failed','paused')),
  origin TEXT NOT NULL DEFAULT 'manual' CHECK(origin IN ('manual','auto','continuation')),
  request_version INTEGER NOT NULL DEFAULT 0 CHECK(request_version>=0),
  attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts>=0),
  due_at TEXT NOT NULL,
  error TEXT,
  updated_at TEXT NOT NULL
);
CREATE TABLE imap_body_requests (
  account_id TEXT NOT NULL,
  message_id TEXT NOT NULL,
  generation TEXT NOT NULL,
  revision INTEGER NOT NULL,
  state TEXT NOT NULL CHECK(state IN ('queued','failed')),
  error TEXT,
  PRIMARY KEY(account_id,message_id),
  FOREIGN KEY(account_id,message_id) REFERENCES messages(account_id,id) ON DELETE CASCADE
);
CREATE TABLE imap_modseq (
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  folder TEXT NOT NULL,
  generation TEXT NOT NULL,
  uid_validity INTEGER NOT NULL CHECK(uid_validity>0),
  highest TEXT NOT NULL,
  PRIMARY KEY(account_id,folder)
);
