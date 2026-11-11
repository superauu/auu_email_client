ALTER TABLE messages ADD COLUMN state_revision INTEGER NOT NULL DEFAULT 0;
ALTER TABLE messages ADD COLUMN remote_folder TEXT;
ALTER TABLE messages ADD COLUMN remote_uid_validity INTEGER;
ALTER TABLE messages ADD COLUMN remote_uid INTEGER;
ALTER TABLE messages ADD COLUMN remote_generation TEXT;
CREATE TABLE message_changes (
  account_id TEXT NOT NULL,
  message_id TEXT NOT NULL,
  field TEXT NOT NULL CHECK(field IN ('unread','starred','archive')),
  value INTEGER NOT NULL CHECK(value IN (0,1)),
  revision INTEGER NOT NULL,
  state TEXT NOT NULL CHECK(state IN ('queued','applying','failed','conflict','unknown','local')),
  generation TEXT NOT NULL,
  source_folder TEXT,
  source_validity INTEGER,
  source_uid INTEGER,
  target_folder TEXT,
  error TEXT,
  updated_at TEXT NOT NULL,
  PRIMARY KEY(account_id,message_id,field),
  FOREIGN KEY(account_id,message_id) REFERENCES messages(account_id,id) ON DELETE CASCADE
);
CREATE INDEX message_changes_account_state ON message_changes(account_id,state);
