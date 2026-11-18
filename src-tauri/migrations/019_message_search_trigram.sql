-- v19：中文 / 任意子串检索升级——FTS5 trigram 分词器（SQLite ≥ 3.34）。
-- unicode61 把中文连续词组当单 token（无法子串匹配），LIKE 是全表扫描；trigram 以
-- 3 字符滑动窗口索引，MATCH 语义即子串匹配，中英文一致。<3 字符的短查询仍回退 LIKE
-- （trigram 无法产生短于 3 字符的查询序列）。外部队列表用 rowid 关联 messages；
-- UPDATE 触发器限定 OF subject,sender_name,sender_email,body_text——未读 / 星标等
-- 标记更新不重索引正文（吸取 017 移除的旧 FTS 写放大教训）。
CREATE VIRTUAL TABLE messages_search USING fts5(subject, sender_name, sender_email, body_text, content='messages', content_rowid='rowid', tokenize='trigram');
INSERT INTO messages_search(rowid, subject, sender_name, sender_email, body_text)
  SELECT rowid, subject, sender_name, sender_email, COALESCE(body_text, '') FROM messages;
CREATE TRIGGER messages_search_insert AFTER INSERT ON messages BEGIN
  INSERT INTO messages_search(rowid, subject, sender_name, sender_email, body_text)
    VALUES (new.rowid, new.subject, new.sender_name, new.sender_email, COALESCE(new.body_text, ''));
END;
CREATE TRIGGER messages_search_delete AFTER DELETE ON messages BEGIN
  INSERT INTO messages_search(messages_search, rowid, subject, sender_name, sender_email, body_text)
    VALUES ('delete', old.rowid, old.subject, old.sender_name, old.sender_email, COALESCE(old.body_text, ''));
END;
CREATE TRIGGER messages_search_update AFTER UPDATE OF subject, sender_name, sender_email, body_text ON messages BEGIN
  INSERT INTO messages_search(messages_search, rowid, subject, sender_name, sender_email, body_text)
    VALUES ('delete', old.rowid, old.subject, old.sender_name, old.sender_email, COALESCE(old.body_text, ''));
  INSERT INTO messages_search(rowid, subject, sender_name, sender_email, body_text)
    VALUES (new.rowid, new.subject, new.sender_name, new.sender_email, COALESCE(new.body_text, ''));
END;
