-- v2: columns needed by the mail repository and paged IPC.
-- recipients: JSON array of addresses; messages in v1 had no recipient column.
ALTER TABLE messages ADD COLUMN recipients_json TEXT NOT NULL DEFAULT '[]';
-- Non-secret account profile (avatar letter, color, status, capability flags) as JSON.
-- Secrets never live here; they belong to the OS credential service.
ALTER TABLE accounts ADD COLUMN profile_json TEXT;
