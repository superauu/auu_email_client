-- An in-flight network response must not write into a deleted/recreated or reconfigured account.
ALTER TABLE accounts ADD COLUMN sync_generation TEXT NOT NULL DEFAULT '';
UPDATE accounts SET sync_generation = lower(hex(randomblob(16)));
CREATE TRIGGER accounts_sync_generation_insert AFTER INSERT ON accounts BEGIN
  UPDATE accounts SET sync_generation = lower(hex(randomblob(16))) WHERE id = new.id;
END;
CREATE TRIGGER accounts_sync_generation_update AFTER UPDATE OF provider, email, profile_json ON accounts
WHEN old.provider IS NOT new.provider OR old.email IS NOT new.email OR old.profile_json IS NOT new.profile_json BEGIN
  UPDATE accounts SET sync_generation = lower(hex(randomblob(16))) WHERE id = new.id;
END;
