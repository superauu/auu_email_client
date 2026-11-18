-- v21：远程图片信任列表（全局，不按账号作用域——发件人身份与接收账号无关）。
-- kind='sender' 信任完整发件地址；kind='domain' 信任发件域名（精确匹配，不含子域）。
-- 信任后阅读区自动加载该来源的远程图片（loading = 允许追踪图，属用户明确选择）。
CREATE TABLE trusted_senders (
  kind TEXT NOT NULL CHECK(kind IN ('sender','domain')),
  value TEXT NOT NULL,
  created_at TEXT NOT NULL,
  PRIMARY KEY (kind, value)
);
