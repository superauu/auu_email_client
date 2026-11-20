/**
 * AI 助手领域契约：OpenAI 兼容服务的三种只读任务（摘要 / 回复草稿 / 提取待办）。
 * AI 输出永远是文本——不能发邮件、改账号或执行任何操作；回复草稿由用户在撰写面
 * 确认后走既有发送流程，提取的待办仅为文本展示。API Key 只存系统凭据库，
 * 绝不进入 localStorage、SQLite 或日志；配置（服务地址 / 模型）存 SQLite v22。
 */
export type AiTaskKind = 'summarize' | 'draft-reply' | 'extract-events';

export interface AiConfig {
  baseUrl: string;
  model: string;
  /** 只回存在性；Key 本身绝不跨 IPC 返回。 */
  keySaved: boolean;
}

export interface AiOutcome {
  text: string;
  model: string;
}
