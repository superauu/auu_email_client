CREATE TABLE recovery_scan (
  account_id TEXT NOT NULL,
  message_id TEXT NOT NULL,
  checkpoint_json TEXT NOT NULL,
  checked INTEGER NOT NULL CHECK(checked >= 0),
  total INTEGER NOT NULL CHECK(total >= checked),
  matches INTEGER NOT NULL CHECK(matches >= 0),
  cross_uid_validity INTEGER NOT NULL CHECK(cross_uid_validity IN (0,1)),
  PRIMARY KEY(account_id,message_id),
  FOREIGN KEY(account_id,message_id) REFERENCES message_recovery(account_id,message_id) ON DELETE CASCADE
);
