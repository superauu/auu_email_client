ALTER TABLE messages ADD COLUMN locator_verified INTEGER NOT NULL DEFAULT 0 CHECK(locator_verified IN (0,1));
ALTER TABLE messages ADD COLUMN content_hash_verified INTEGER NOT NULL DEFAULT 0 CHECK(content_hash_verified IN (0,1));
-- A cached-fields recovery never proves historical raw bytes or an old account binding.
UPDATE messages SET locator_verified=1
WHERE remote_generation=(SELECT sync_generation FROM accounts WHERE id=messages.account_id)
  AND remote_folder IS NOT NULL AND remote_uid_validity>0 AND remote_uid>0
  AND NOT EXISTS(SELECT 1 FROM message_recovery r WHERE r.message_id=messages.id);
UPDATE messages SET content_hash_verified=1 WHERE locator_verified=1 AND length(content_hash)=64;
ALTER TABLE message_recovery ADD COLUMN operation TEXT NOT NULL DEFAULT 'recovery' CHECK(operation IN ('recovery','fingerprint'));
ALTER TABLE recovery_scan ADD COLUMN segment_progress_json TEXT;
