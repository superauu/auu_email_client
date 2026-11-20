-- AI 服务配置（全局单行；API Key 存系统凭据服务 provider='ai'，绝不入库）。
-- base_url / model 是 OpenAI 兼容端点的非机密配置，供 ai_run / ai_test 使用。
CREATE TABLE IF NOT EXISTS ai_settings (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  base_url TEXT NOT NULL,
  model TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
