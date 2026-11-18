-- v20：每账号邮件签名（Outlook 式自动追加）。html 为 compose 消毒后的签名正文，
-- 图片以 data:image/* URI 内嵌（发送时由 outbox 转为 cid: 内联件）；不在 profile_json
-- 上扩展——004 的 accounts_update 触发器会把 profile_json 变更旋转 sync_generation，
-- 签名保存不应被视为账号重配。开关独立控制新邮件与回复 / 转发的追加行为。
CREATE TABLE account_signatures (
  account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
  html TEXT NOT NULL DEFAULT '',
  use_new INTEGER NOT NULL DEFAULT 1 CHECK(use_new IN (0,1)),
  use_reply INTEGER NOT NULL DEFAULT 1 CHECK(use_reply IN (0,1)),
  updated_at TEXT NOT NULL
);
