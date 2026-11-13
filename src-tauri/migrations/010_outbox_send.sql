-- v10：SMTP 发送。outbox 表自 v1 已存在（状态机 queued/sending/sent/failed/unknown 与
-- docs/SENDING.md 一致）；这里补充发送过程列。Message-ID 登记时生成一次全程复用，
-- attempts 记录真实投递尝试次数；unknown 的出口只有用户明确确认或 Sent 目录核对命中。
ALTER TABLE outbox ADD COLUMN message_id TEXT;
ALTER TABLE outbox ADD COLUMN attempts INTEGER NOT NULL DEFAULT 0;
ALTER TABLE outbox ADD COLUMN last_error TEXT;
ALTER TABLE outbox ADD COLUMN queued_at TEXT;
ALTER TABLE outbox ADD COLUMN resolved_by TEXT;

CREATE INDEX outbox_account_state ON outbox(account_id, state);
