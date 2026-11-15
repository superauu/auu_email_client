-- 回复正确性：持久化 Cc、Reply-To 头地址与 References 会话链。
-- cc_json / reply_to_json 与 recipients_json 同构（地址数组）；header_references 为原邮件 References 头原文（可空）。
ALTER TABLE messages ADD COLUMN cc_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE messages ADD COLUMN reply_to_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE messages ADD COLUMN header_references TEXT;
