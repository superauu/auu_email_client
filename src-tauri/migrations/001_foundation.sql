-- UTC ISO-8601 strings; UUID/string IDs are opaque to callers. No credential columns.
CREATE TABLE accounts (
  id TEXT PRIMARY KEY,
  provider TEXT NOT NULL CHECK(provider IN ('imap', 'pop3', 'microsoft', 'gmail')),
  email TEXT NOT NULL,
  display_name TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE TABLE folders (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  remote_id TEXT NOT NULL,
  name TEXT NOT NULL,
  kind TEXT NOT NULL,
  UNIQUE(account_id, remote_id),
  UNIQUE(account_id, id)
);
CREATE TABLE messages (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  remote_id TEXT NOT NULL,
  thread_id TEXT NOT NULL,
  subject TEXT NOT NULL,
  sender_name TEXT NOT NULL,
  sender_email TEXT NOT NULL,
  body_text TEXT,
  received_at TEXT NOT NULL,
  unread INTEGER NOT NULL DEFAULT 1 CHECK(unread IN (0, 1)),
  starred INTEGER NOT NULL DEFAULT 0 CHECK(starred IN (0, 1)),
  UNIQUE(account_id, remote_id),
  UNIQUE(account_id, id)
);
-- A message may belong to several provider folders/labels; no single-folder assumption.
CREATE TABLE message_folders (
  account_id TEXT NOT NULL,
  message_id TEXT NOT NULL,
  folder_id TEXT NOT NULL,
  PRIMARY KEY(account_id, message_id, folder_id),
  FOREIGN KEY(account_id, message_id) REFERENCES messages(account_id, id) ON DELETE CASCADE,
  FOREIGN KEY(account_id, folder_id) REFERENCES folders(account_id, id) ON DELETE CASCADE
);
CREATE INDEX messages_account_date ON messages(account_id, received_at DESC);
CREATE TABLE labels (
  id TEXT PRIMARY KEY,
  account_id TEXT REFERENCES accounts(id) ON DELETE CASCADE,
  remote_id TEXT,
  name TEXT NOT NULL,
  color TEXT NOT NULL,
  scope TEXT NOT NULL CHECK(scope IN ('local', 'server')),
  CHECK(scope = 'local' OR (account_id IS NOT NULL AND remote_id IS NOT NULL))
);
CREATE TABLE message_labels (
  message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  label_id TEXT NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
  PRIMARY KEY(message_id, label_id)
);
CREATE TABLE drafts (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  recipients_json TEXT NOT NULL DEFAULT '[]',
  subject TEXT NOT NULL DEFAULT '',
  body_text TEXT NOT NULL DEFAULT '',
  reply_to_id TEXT,
  updated_at TEXT NOT NULL
);
CREATE TABLE mail_events (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  message_id TEXT,
  title TEXT NOT NULL,
  starts_at TEXT NOT NULL,
  time_zone TEXT NOT NULL,
  completed INTEGER NOT NULL DEFAULT 0 CHECK(completed IN (0, 1)),
  source TEXT NOT NULL CHECK(source IN ('local', 'ics', 'provider')),
  FOREIGN KEY(account_id, message_id) REFERENCES messages(account_id, id)
);
CREATE TRIGGER detach_events_before_message_delete BEFORE DELETE ON messages BEGIN
  UPDATE mail_events SET message_id = NULL WHERE account_id = old.account_id AND message_id = old.id;
END;
CREATE TABLE sync_states (
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  scope TEXT NOT NULL,
  cursor TEXT,
  updated_at TEXT NOT NULL,
  PRIMARY KEY(account_id, scope)
);
CREATE TABLE outbox (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  draft_id TEXT NOT NULL REFERENCES drafts(id),
  state TEXT NOT NULL CHECK(state IN ('queued', 'sending', 'sent', 'failed', 'unknown')),
  idempotency_key TEXT NOT NULL UNIQUE,
  updated_at TEXT NOT NULL
);
CREATE VIRTUAL TABLE messages_fts USING fts5(subject, sender_name, sender_email, body_text, content='messages', content_rowid='rowid');
CREATE TRIGGER messages_fts_insert AFTER INSERT ON messages BEGIN
  INSERT INTO messages_fts(rowid, subject, sender_name, sender_email, body_text) VALUES (new.rowid, new.subject, new.sender_name, new.sender_email, new.body_text);
END;
CREATE TRIGGER messages_fts_delete AFTER DELETE ON messages BEGIN
  INSERT INTO messages_fts(messages_fts, rowid, subject, sender_name, sender_email, body_text) VALUES ('delete', old.rowid, old.subject, old.sender_name, old.sender_email, old.body_text);
END;
CREATE TRIGGER messages_fts_update AFTER UPDATE ON messages BEGIN
  INSERT INTO messages_fts(messages_fts, rowid, subject, sender_name, sender_email, body_text) VALUES ('delete', old.rowid, old.subject, old.sender_name, old.sender_email, old.body_text);
  INSERT INTO messages_fts(rowid, subject, sender_name, sender_email, body_text) VALUES (new.rowid, new.subject, new.sender_name, new.sender_email, new.body_text);
END;
