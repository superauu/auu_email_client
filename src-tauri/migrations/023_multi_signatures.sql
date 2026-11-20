-- 每账号多签名（v20 的单签名升级）：签名列表 + 每账号唯一默认签名。
-- 既有单签名迁移为一条名为「默认签名」的条目并置为默认；空签名不迁移（等于未设置）。
CREATE TABLE IF NOT EXISTS signatures (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  html TEXT NOT NULL DEFAULT '',
  use_new INTEGER NOT NULL DEFAULT 1 CHECK (use_new IN (0,1)),
  use_reply INTEGER NOT NULL DEFAULT 1 CHECK (use_reply IN (0,1)),
  is_default INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0,1)),
  updated_at TEXT NOT NULL
);
INSERT INTO signatures (id, account_id, name, html, use_new, use_reply, is_default, updated_at)
  SELECT account_id, account_id, '默认签名', html, use_new, use_reply, 1, updated_at
  FROM account_signatures WHERE html != '';
DROP TABLE account_signatures;
