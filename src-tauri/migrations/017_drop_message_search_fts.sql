-- 全文检索改为 LIKE 子串查询（与 UI 选择器的 subject / 发件人 / 正文子串语义一致，
-- FTS5 unicode61 分词器把中文连续词组当作单 token，无法支撑中文搜索）。
-- 同时移除维护性触发器：大邮箱同步时每行 messages UPDATE 都会重索引整个正文，是不必要的固定开销。
DROP TRIGGER IF EXISTS messages_fts_insert;
DROP TRIGGER IF EXISTS messages_fts_delete;
DROP TRIGGER IF EXISTS messages_fts_update;
DROP TABLE IF EXISTS messages_fts;
