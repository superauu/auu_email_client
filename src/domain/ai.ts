import type { Timestamp } from './mail';

export type AiTaskKind = 'summarize' | 'draft-reply' | 'translate' | 'extract-events' | 'suggest-labels';
export interface AiRequest {
  task: AiTaskKind;
  accountId: string;
  messageIds: string[];
  /** User-selected text only. Mail is untrusted input, never tool instructions. */
  contextText: string;
  instruction?: string;
}
export interface AiResult {
  text: string;
  model: string;
  createdAt: Timestamp;
}
export interface AiProvider {
  id: string;
  execute(request: AiRequest, options: { signal: AbortSignal; onText?: (delta: string) => void }): Promise<AiResult>;
}
// AI cannot send mail, mutate accounts, or execute actions through this interface.
