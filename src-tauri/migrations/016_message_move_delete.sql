-- v16：扩展 message_changes.field 允许 delete（删除到废纸篓）与 move（移动到文件夹）。
-- SQLite 无法修改 CHECK 约束：按约定重建表并原样迁移既有意图，不改写历史迁移。
CREATE TABLE message_changes_v16 (
  account_id TEXT NOT NULL,
  message_id TEXT NOT NULL,
  field TEXT NOT NULL CHECK(field IN ('unread','starred','archive','delete','move')),
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
INSERT INTO message_changes_v16 (
  account_id,message_id,field,value,revision,state,generation,source_folder,source_validity,source_uid,target_folder,error,updated_at
)
SELECT
  account_id,message_id,field,value,revision,state,generation,source_folder,source_validity,source_uid,target_folder,error,updated_at
FROM message_changes;
DROP TABLE message_changes;
ALTER TABLE message_changes_v16 RENAME TO message_changes;
CREATE INDEX message_changes_account_state ON message_changes(account_id,state);
