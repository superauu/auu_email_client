-- 撰写附件：元数据数组保存在 drafts 行（有序、单一事实来源），
-- 字节内容入子表（保存事务内从 Rust 暂存区写入；随草稿删除级联清理）。
ALTER TABLE drafts ADD COLUMN attachments_json TEXT NOT NULL DEFAULT '[]';

CREATE TABLE draft_attachment_blobs (
  draft_id TEXT NOT NULL REFERENCES drafts(id) ON DELETE CASCADE,
  attachment_id TEXT NOT NULL,
  sha256 TEXT NOT NULL,
  content BLOB NOT NULL,
  PRIMARY KEY (draft_id, attachment_id)
);
