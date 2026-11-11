ALTER TABLE messages ADD COLUMN content_hash TEXT;
ALTER TABLE messages ADD COLUMN header_message_id TEXT;
CREATE TABLE message_recovery (
  account_id TEXT NOT NULL,
  message_id TEXT NOT NULL,
  generation TEXT NOT NULL,
  revision INTEGER NOT NULL,
  action TEXT NOT NULL CHECK(action IN ('check','confirm','retry-archive')),
  state TEXT NOT NULL CHECK(state IN ('queued','checking','located','source-present','unresolved','failed','restored','local')),
  detail TEXT NOT NULL,
  proof_json TEXT,
  updated_at TEXT NOT NULL,
  PRIMARY KEY(account_id,message_id),
  FOREIGN KEY(account_id,message_id) REFERENCES messages(account_id,id) ON DELETE CASCADE
);
CREATE INDEX message_recovery_pending ON message_recovery(account_id,state);
